use super::*;
use crate::gacos_result_worker::{
    GacosResultError, GacosResultFetchEvidence, GacosResultLinkCache, GacosResultTransport,
};
use crate::{GacosCreateResultTaskRequest, GacosSupplyResultLinkRequest};
use std::path::Path;

const LINK: &str =
    "https://www.gacos.net/results/private-fixture-secret.tar?signature=fixture-only";
struct ResultFixtureTransport {
    calls: Arc<AtomicUsize>,
    fail: bool,
}
impl GacosResultTransport for ResultFixtureTransport {
    fn fetch(
        &self,
        _: &str,
        _: &str,
        _: &str,
        raw: &str,
        destination: &Path,
        _: &WorkerControl,
        _: &mut dyn FnMut(DownloadItemProgress),
    ) -> Result<GacosResultFetchEvidence, GacosResultError> {
        assert_eq!(raw, LINK);
        self.calls.fetch_add(1, Ordering::SeqCst);
        if self.fail {
            return Err(GacosResultError::TransferFailed);
        }
        let mut archive = tar::Builder::new(Vec::new());
        for (name, bytes) in [
            ("20240101.ztd", vec![0u8; 64]),
            (
                "20240101.ztd.rsc",
                b"WIDTH 4\nFILE_LENGTH 4\nX_FIRST 99\nY_FIRST 32\nX_STEP 1\nY_STEP -1\n".to_vec(),
            ),
        ] {
            let mut header = tar::Header::new_gnu();
            header.set_size(bytes.len() as u64);
            header.set_mode(0o600);
            header.set_cksum();
            archive
                .append_data(&mut header, name, bytes.as_slice())
                .unwrap();
        }
        let bytes = archive.into_inner().unwrap();
        std::fs::write(destination, &bytes).unwrap();
        Ok(GacosResultFetchEvidence {
            size_bytes: bytes.len() as u64,
            etag: "\"fixture-v1\"".to_owned(),
            bytes_written: bytes.len() as u64,
            reused_existing: false,
        })
    }
}
fn result_service(fixture: &Fixture, calls: &Arc<AtomicUsize>, fail: bool) -> ApplicationService {
    let mut service = fixture.service();
    service.gacos_result_links = Arc::new(GacosResultLinkCache::with_transport(Arc::new(
        ResultFixtureTransport {
            calls: Arc::clone(calls),
            fail,
        },
    )));
    service
}
fn result_request(fixture: &Fixture, service: &ApplicationService) -> GacosCreateResultTaskRequest {
    result_request_with_format(fixture, service, None)
}
fn result_request_with_format(
    fixture: &Fixture,
    service: &ApplicationService,
    output_format: Option<&str>,
) -> GacosCreateResultTaskRequest {
    let request = fixture.prepare_with_format(service, 1, output_format);
    let parent = service.create_gacos_request_task(&request).unwrap();
    GacosCreateResultTaskRequest {
        task_id: "gacos-email-result".to_owned(),
        source_request_task_id: request.task_id,
        source_request_item_id: parent.snapshot.candidates[0].item_id.clone(),
        result_url: LINK.to_owned(),
    }
}
fn assert_private(value: &impl serde::Serialize) {
    let text = serde_json::to_string(value).unwrap();
    for secret in [LINK, "private-fixture-secret", "signature=fixture-only"] {
        assert!(
            !text.contains(secret),
            "raw link leaked into persisted state"
        );
    }
}
#[test]
fn manual_result_task_downloads_admits_archives_and_restores_without_link_leak_or_repost() {
    let fixture = Fixture::new();
    let calls = Arc::new(AtomicUsize::new(0));
    let service = result_service(&fixture, &calls, false);
    let request = result_request(&fixture, &service);
    let created = service.gacos_create_result_task(&request).unwrap();
    assert_eq!(created["source"], "native-gacos-manual-result");
    assert_private(&created);
    let terminal = await_import_terminal(&service, &request.task_id);
    let store = service.store_handle().unwrap();
    let download = store
        .lock()
        .unwrap()
        .load_download_snapshot(&request.task_id)
        .unwrap();
    assert_eq!(
        terminal.runtime.status,
        insar_core::TaskLifecycleStatus::Completed,
        "{download:?}"
    );
    assert_eq!(calls.load(Ordering::SeqCst), 1);
    assert_private(&terminal.snapshot);
    assert_private(&download);
    assert_eq!(
        terminal.snapshot.output_root,
        fixture.root.join("output/gacos-results")
    );
    assert_eq!(
        std::fs::read(download.results[0].destination.join("20240101.ztd")).unwrap(),
        vec![0u8; 64]
    );
    let parent = store
        .lock()
        .unwrap()
        .load_task_record(&request.source_request_task_id)
        .unwrap()
        .unwrap();
    assert_eq!(
        parent.runtime.status,
        insar_core::TaskLifecycleStatus::Paused
    );
    assert!(store
        .lock()
        .unwrap()
        .read_gacos_submission_attempts(&request.source_request_task_id, None, 8)
        .unwrap()
        .is_empty());
    store
        .lock()
        .unwrap()
        .archive_task(&StoreArchiveTaskRequest {
            task_id: request.task_id.clone(),
            expected_revision: terminal.runtime.revision,
            archived_at: Utc::now().to_rfc3339_opts(SecondsFormat::Millis, true),
        })
        .unwrap();
    drop(store);
    service.shutdown().unwrap();
    let reopened = result_service(&fixture, &calls, false);
    let store = reopened.store_handle().unwrap();
    assert!(store
        .lock()
        .unwrap()
        .load_scheduler_tasks()
        .unwrap()
        .iter()
        .all(|task| task.snapshot.task_id != request.task_id));
    let history = store.lock().unwrap().list_history().unwrap();
    assert!(history
        .iter()
        .any(|entry| entry.task.snapshot == terminal.snapshot));
    for entry in &history {
        assert_private(&entry.task.snapshot);
        assert_private(&entry.task.runtime);
    }
    assert_eq!(calls.load(Ordering::SeqCst), 1);
    reopened.shutdown().unwrap();
}

#[test]
fn manual_result_task_accepts_binary_parent_without_copying_request_only_fields() {
    let fixture = Fixture::new();
    let calls = Arc::new(AtomicUsize::new(0));
    let service = result_service(&fixture, &calls, false);
    let request = result_request_with_format(&fixture, &service, Some("binary"));
    let created = service.gacos_create_result_task(&request).unwrap();
    assert_eq!(created["source"], "native-gacos-manual-result");
    let child = service
        .store_handle()
        .unwrap()
        .lock()
        .unwrap()
        .load_task_record(&request.task_id)
        .unwrap()
        .unwrap();
    let manifest = child.snapshot.candidates[0]
        .payload
        .get("gacos_result_manifest")
        .unwrap();
    assert!(manifest.get("output_format").is_none());
    assert_eq!(
        manifest["source_request_task_id"].as_str(),
        Some(request.source_request_task_id.as_str())
    );
    assert_eq!(
        manifest["source_request_item_id"].as_str(),
        Some(request.source_request_item_id.as_str())
    );
    service.shutdown().unwrap();
}

fn retry_result(service: &ApplicationService, task_id: &str) {
    let task = service
        .store_handle()
        .unwrap()
        .lock()
        .unwrap()
        .load_task_record(task_id)
        .unwrap()
        .unwrap();
    service
        .update_download_task(&StoreUpdateTaskRequest {
            task_id: task_id.to_owned(),
            expected_revision: task.runtime.revision,
            event_id: format!("result-retry-{}", task.runtime.revision),
            occurred_at: Utc::now().to_rfc3339_opts(SecondsFormat::Millis, true),
            command: StoreTaskCommand {
                kind: TaskStateCommandKind::RetryItems,
                item_ids: task.snapshot.initial_selected_item_ids,
            },
        })
        .unwrap();
}
#[test]
fn manual_result_retry_after_restart_requires_exact_link_and_preserves_frozen_request() {
    let fixture = Fixture::new();
    let calls = Arc::new(AtomicUsize::new(0));
    let service = result_service(&fixture, &calls, true);
    let request = result_request(&fixture, &service);
    service.gacos_create_result_task(&request).unwrap();
    let failed = await_import_terminal(&service, &request.task_id);
    assert_eq!(
        failed.runtime.status,
        insar_core::TaskLifecycleStatus::Failed
    );
    service.shutdown().unwrap();
    let reopened = result_service(&fixture, &calls, false);
    retry_result(&reopened, &request.task_id);
    let no_link = await_import_terminal(&reopened, &request.task_id);
    let store = reopened.store_handle().unwrap();
    let download = store
        .lock()
        .unwrap()
        .load_download_snapshot(&request.task_id)
        .unwrap();
    assert_eq!(
        download.results[0].failure.as_ref().unwrap().code,
        "GACOS_RESULT_LINK_REQUIRED"
    );
    assert_eq!(calls.load(Ordering::SeqCst), 1);
    assert_eq!(no_link.snapshot, failed.snapshot);
    assert!(reopened
        .gacos_supply_result_link(&GacosSupplyResultLinkRequest {
            task_id: request.task_id.clone(),
            result_url: LINK.replace("fixture-only", "wrong-fixture")
        })
        .is_err());
    reopened
        .gacos_supply_result_link(&GacosSupplyResultLinkRequest {
            task_id: request.task_id.clone(),
            result_url: LINK.to_owned(),
        })
        .unwrap();
    let unchanged = store
        .lock()
        .unwrap()
        .load_task_record(&request.task_id)
        .unwrap()
        .unwrap();
    assert_eq!(unchanged.runtime.revision, no_link.runtime.revision);
    assert_eq!(
        unchanged.runtime.status,
        insar_core::TaskLifecycleStatus::Failed
    );
    retry_result(&reopened, &request.task_id);
    let terminal = await_import_terminal(&reopened, &request.task_id);
    assert_eq!(
        terminal.runtime.status,
        insar_core::TaskLifecycleStatus::Completed,
        "{:?}",
        store
            .lock()
            .unwrap()
            .load_download_snapshot(&request.task_id)
            .unwrap()
    );
    assert_eq!(terminal.snapshot, failed.snapshot);
    assert_eq!(calls.load(Ordering::SeqCst), 2);
    assert!(store
        .lock()
        .unwrap()
        .read_gacos_submission_attempts(&request.source_request_task_id, None, 8)
        .unwrap()
        .is_empty());
    reopened.shutdown().unwrap();
}
#[test]
fn manual_result_rejects_foreign_parent_and_unreviewed_host_before_transfer() {
    let fixture = Fixture::new();
    let calls = Arc::new(AtomicUsize::new(0));
    let service = result_service(&fixture, &calls, false);
    let mut request = result_request(&fixture, &service);
    request.source_request_item_id = "foreign-group".to_owned();
    assert!(service.gacos_create_result_task(&request).is_err());
    let parent = service
        .store_handle()
        .unwrap()
        .lock()
        .unwrap()
        .load_task_record(&request.source_request_task_id)
        .unwrap()
        .unwrap();
    request.source_request_item_id = parent.snapshot.candidates[0].item_id.clone();
    request.result_url = "https://unreviewed.example.invalid/result.tar".to_owned();
    assert!(service.gacos_create_result_task(&request).is_err());
    assert_eq!(calls.load(Ordering::SeqCst), 0);
    assert!(service
        .store_handle()
        .unwrap()
        .lock()
        .unwrap()
        .load_task_record(&request.task_id)
        .unwrap()
        .is_none());
    service.shutdown().unwrap();
}
