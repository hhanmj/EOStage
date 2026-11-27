use std::fs;
use std::io::{BufRead, BufReader, Write};
use std::net::TcpListener;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant};

use insar_core::{
    AssetRole, DownloadLayout, FrozenSearchBatch, SearchWorkbenchState, StoreCreateTaskRequest,
    StoreSearchBatchRequest, TaskLifecycleStatus,
};
use insar_store::{StoreLocalReuseDisposition, TaskStore};
use serde_json::json;

use super::tests::{
    burst_loopback_worker, burst_worker_item, BURST_NATIVE_PRODUCT_ID, BURST_TIFF_NAME,
};
use crate::TransferOptions;

/// #23: the burst extractor delivers a GeoTIFF larger than the catalogue's
/// raw burst length. A later request for the same burst reuses the file the
/// landing ledger recorded instead of renaming it aside and downloading it
/// again.
#[test]
fn a_landed_burst_tiff_larger_than_its_catalogue_size_is_reused_from_the_ledger() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    listener.set_nonblocking(true).unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    let tiff_requests = Arc::new(AtomicUsize::new(0));
    let stop = Arc::new(AtomicBool::new(false));
    let mut tiff = vec![0_u8; 1_100];
    tiff[..14].copy_from_slice(b"II*\0\x08\0\0\0\0\0\0\0\0\0");
    let delivered = tiff.clone();
    let server = {
        let tiff_requests = Arc::clone(&tiff_requests);
        let stop = Arc::clone(&stop);
        thread::spawn(move || {
            while !stop.load(Ordering::SeqCst) {
                let (stream, _) = match listener.accept() {
                    Ok(value) => value,
                    Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                        thread::sleep(Duration::from_millis(10));
                        continue;
                    }
                    Err(error) => panic!("loopback accept failed: {error}"),
                };
                stream.set_nonblocking(false).unwrap();
                let mut reader = BufReader::new(stream);
                let mut request_line = String::new();
                reader.read_line(&mut request_line).unwrap();
                loop {
                    let mut line = String::new();
                    reader.read_line(&mut line).unwrap();
                    if line == "\r\n" || line.is_empty() {
                        break;
                    }
                }
                let mut stream = reader.into_inner();
                if request_line.starts_with("POST /token ") {
                    let body = br#"{"access_token":"fixture-token"}"#;
                    write!(
                        stream,
                        "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                        body.len()
                    )
                    .unwrap();
                    stream.write_all(body).unwrap();
                } else if request_line.starts_with("GET /authorize ") {
                    write!(
                        stream,
                        "HTTP/1.1 200 OK\r\nSet-Cookie: asf-urs=fixture-session; Path=/; HttpOnly\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
                    )
                    .unwrap();
                } else if request_line.starts_with("GET /measurement.tiff ") {
                    tiff_requests.fetch_add(1, Ordering::SeqCst);
                    write!(
                        stream,
                        "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                        tiff.len()
                    )
                    .unwrap();
                    stream.write_all(&tiff).unwrap();
                } else {
                    write!(
                        stream,
                        "HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
                    )
                    .unwrap();
                }
                stream.flush().unwrap();
            }
        })
    };

    let nonce = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let root = std::env::temp_dir().join(format!(
        "insar-burst-ledger-reuse-{}-{nonce}",
        std::process::id()
    ));
    fs::create_dir_all(&root).unwrap();
    let output_root = root.join("out");
    let database = root.join("store.sqlite3");
    let item = burst_worker_item(AssetRole::Primary);
    assert_eq!(item.payload["file_size_remote"], json!(1_024));
    let seed = |store: &mut TaskStore, task_id: &str| {
        let batch_id = format!("batch-{task_id}");
        store
            .save_search_batch(&StoreSearchBatchRequest {
                activate_workbench: true,
                batch: FrozenSearchBatch {
                    schema_version: 1,
                    batch_id: batch_id.clone(),
                    provider_id: "asf.catalog".to_owned(),
                    created_at: "2026-10-22T00:00:00Z".to_owned(),
                    aoi_name: "fixture".to_owned(),
                    aoi_geojson: None,
                    query: json!({"dataset": "SLC-BURST"}).as_object().unwrap().clone(),
                    search_stats: None,
                    candidates: vec![item.clone()],
                },
                workbench: SearchWorkbenchState {
                    batch_id: batch_id.clone(),
                    checked_item_ids: vec![],
                    page_index: 0,
                    page_size: 24,
                    revision: 0,
                },
            })
            .unwrap();
        store
            .create_task(&StoreCreateTaskRequest {
                task_id: task_id.to_owned(),
                batch_id,
                selected_item_ids: vec![item.item_id.clone()],
                checked_item_ids: vec![],
                page_index: 0,
                page_size: 24,
                output_root: output_root.clone(),
                layout: DownloadLayout::Direct,
                concurrency: 1,
                created_at: "2026-10-22T00:00:01Z".to_owned(),
            })
            .unwrap();
    };
    let wait_until = |task_id: &str, status: TaskLifecycleStatus| {
        let deadline = Instant::now() + Duration::from_secs(30);
        loop {
            let task = TaskStore::open(&database)
                .unwrap()
                .load_task(task_id)
                .unwrap()
                .unwrap();
            if task.runtime.status == status {
                return;
            }
            assert!(
                Instant::now() < deadline,
                "{task_id} did not reach {status:?}: {:?}",
                task.runtime.status
            );
            thread::sleep(Duration::from_millis(20));
        }
    };

    seed(
        &mut TaskStore::open(&database).unwrap(),
        "burst-ledger-first",
    );
    let (worker, _admission) = burst_loopback_worker(
        &base,
        TransferOptions {
            max_attempts: 1,
            backoff: Duration::ZERO,
            timeout: Duration::from_secs(2),
            ..TransferOptions::default()
        },
    );
    let scheduler =
        crate::DownloadScheduler::start(&database, Arc::new(worker), Duration::from_millis(5))
            .unwrap();
    scheduler.wake();
    wait_until("burst-ledger-first", TaskLifecycleStatus::Completed);
    let destination = output_root
        .join(BURST_NATIVE_PRODUCT_ID)
        .join(BURST_TIFF_NAME);
    {
        let mut store = TaskStore::open(&database).unwrap();
        let ledger = store
            .current_landed_artifact(&destination.to_string_lossy())
            .unwrap()
            .expect("the landing is recorded");
        assert_eq!(ledger.size_bytes, delivered.len() as u64);
        seed(&mut store, "burst-ledger-second");
    }
    scheduler.wake();
    wait_until("burst-ledger-second", TaskLifecycleStatus::Completed);
    scheduler.shutdown();
    stop.store(true, Ordering::SeqCst);
    server.join().unwrap();

    assert_eq!(
        tiff_requests.load(Ordering::SeqCst),
        1,
        "the second task downloaded the burst again"
    );
    let store = TaskStore::open(&database).unwrap();
    let decision = store
        .task_item_local_reuse("burst-ledger-second", &item.item_id)
        .unwrap()
        .expect("the reuse decision is recorded");
    assert_eq!(
        decision.disposition,
        StoreLocalReuseDisposition::Reused {
            grade: "ledger_identity".to_owned()
        }
    );
    assert_eq!(fs::read(&destination).unwrap(), delivered);
    let conflict_copies = fs::read_dir(destination.parent().unwrap())
        .unwrap()
        .filter(|entry| {
            entry
                .as_ref()
                .unwrap()
                .file_name()
                .to_string_lossy()
                .contains(".eostage-conflict-")
        })
        .count();
    assert_eq!(conflict_copies, 0);
    drop(store);
    let _ = fs::remove_dir_all(&root);
}
