use std::fs;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use insar_application_service::{
    ApplicationBindRegionAoiRequest, ApplicationService, ApplicationServiceConfig,
    CapabilityRuntimeObservations, DemConversionRequest, DemOperationPort, DemTaskOperation,
    DemTaskPlanRequest, DemTaskRequest, FrozenOrbitAssociationV1, OrbitFileMetadata, OrbitPlatform,
    OrbitProductType, OrbitTaskRequest, StandaloneDemAoiTaskRequest, StandaloneDemTaskRequest,
    StoreDemConversionSourceRequest, StoreDemSourceIdentity, WorkspaceNamedNodeRequest,
    WorkspaceOpenRequest,
};
use insar_core::{
    AsfRemoteSearchRequest, AsfRemoteSearchResponse, BuildCapabilityState, ContractError,
    DownloadItemProgress, DownloadItemResult, DownloadOutcome, ProviderResult,
    RuntimeReadinessState, StoreArchiveTaskRequest,
};
use insar_download::{DownloadWork, DownloadWorker, WorkerControl};
use insar_service::LiveAsfSearch;
use serde_json::{json, Map, Value};

struct TempStore {
    root: PathBuf,
    database: PathBuf,
}

impl TempStore {
    fn new(label: &str) -> Self {
        let root = std::env::temp_dir().join(format!(
            "insar-rh1-{label}-{}-{}",
            std::process::id(),
            Instant::now().elapsed().as_nanos()
        ));
        fs::create_dir_all(&root).unwrap();
        Self {
            database: root.join("tasks.sqlite3"),
            root,
        }
    }
}

impl Drop for TempStore {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}

#[derive(Default)]
struct LoopbackCompatibilityPort {
    operations: Mutex<Vec<String>>,
    payloads: Mutex<Vec<Value>>,
}

impl DemOperationPort for LoopbackCompatibilityPort {
    fn call(&self, operation: &str, payload: Value) -> Result<Value, String> {
        self.operations.lock().unwrap().push(operation.to_owned());
        self.payloads.lock().unwrap().push(payload);
        Ok(json!({"ok": true, "operation": operation, "mocked": true}))
    }
}

struct EmptySearch;

impl LiveAsfSearch for EmptySearch {
    fn search(
        &self,
        _request: &AsfRemoteSearchRequest,
        _cancelled: &(dyn Fn() -> bool + Sync),
    ) -> ProviderResult<AsfRemoteSearchResponse> {
        Err(ContractError::new("RH1_FIXTURE_ONLY", "loopback", Map::new()).unwrap())
    }
}

struct LoopbackDownloadWorker;

impl DownloadWorker for LoopbackDownloadWorker {
    fn execute(
        &self,
        work: DownloadWork,
        _control: WorkerControl,
        _progress: &mut dyn FnMut(DownloadItemProgress),
    ) -> DownloadItemResult {
        if work.task.provider_id == "sentinel1.orbit" {
            assert!(work.task.orbit_association.is_some());
        }
        DownloadItemResult {
            task_id: work.task.task_id,
            item_id: work.item.item_id,
            outcome: DownloadOutcome::Completed,
            destination: work.task.output_root,
            bytes_written: 0,
            attempt_count: 1,
            failure: None,
            finished_at: "2026-08-11T00:00:02Z".to_owned(),
        }
    }
}

fn geometry() -> Map<String, Value> {
    json!({
        "type": "Polygon",
        "coordinates": [[[100.0, 30.0], [101.0, 30.0], [101.0, 31.0], [100.0, 30.0]]]
    })
    .as_object()
    .unwrap()
    .clone()
}

fn service(temp: &TempStore) -> (ApplicationService, Arc<LoopbackCompatibilityPort>, String) {
    let port = Arc::new(LoopbackCompatibilityPort::default());
    let service = ApplicationService::with_fixture_business_runtime(
        ApplicationServiceConfig::business_store(&temp.database)
            .with_capability_runtime_observations(
                CapabilityRuntimeObservations::default()
                    .with_build(
                        "dem-provider.copernicus-aws.v1",
                        BuildCapabilityState::Available,
                    )
                    .with_runtime(
                        "dem-provider.copernicus-aws.v1",
                        RuntimeReadinessState::Ready,
                    )
                    .with_build(
                        "dem-provider.user-local.v1",
                        BuildCapabilityState::Available,
                    )
                    .with_runtime("dem-provider.user-local.v1", RuntimeReadinessState::Ready)
                    .with_runtime(
                        "orbit-download.asf-sentinel-1.v1",
                        RuntimeReadinessState::Ready,
                    ),
            ),
        Arc::new(EmptySearch),
        Arc::new(LoopbackDownloadWorker),
    )
    .attach_dem_operation_port(Arc::clone(&port) as Arc<dyn DemOperationPort>);
    service
        .workspace_open(&WorkspaceOpenRequest {
            request_id: "rh1-workspace-open".to_owned(),
            workspace_id: "rh1-workspace".to_owned(),
            workspace_root: temp.root.clone(),
            workspace_name: "RH1".to_owned(),
            expected_revision: None,
        })
        .unwrap();
    let project = service
        .project_create(&WorkspaceNamedNodeRequest {
            request_id: "rh1-project-create".to_owned(),
            parent_id: "rh1-workspace".to_owned(),
            expected_revision: 1,
            name: "RH1".to_owned(),
        })
        .unwrap();
    let project_id = project.project_id.clone().unwrap();
    let region = service
        .region_create(&WorkspaceNamedNodeRequest {
            request_id: "rh1-region-create".to_owned(),
            parent_id: project_id,
            expected_revision: project.revision,
            name: "RH1 region".to_owned(),
        })
        .unwrap();
    let region_id = region.region_id.clone().unwrap();
    service
        .bind_region_aoi(&ApplicationBindRegionAoiRequest {
            request_id: "rh1-bind-aoi".to_owned(),
            region_id: region_id.clone(),
            expected_revision: None,
            source_kind: "manual_bbox".to_owned(),
            geometry: geometry(),
            bbox: vec![100.0, 30.0, 101.0, 31.0],
            crs: "EPSG:4326".to_owned(),
            display_name: "RH1 region".to_owned(),
            bound_at: "2026-08-11T00:00:00Z".to_owned(),
        })
        .unwrap();
    (service, port, region_id)
}

fn wait_for_completion(service: &ApplicationService, task_id: &str) -> u64 {
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        let snapshot = service.download_snapshot(task_id).unwrap();
        if !snapshot.results.is_empty() {
            assert_eq!(snapshot.results[0].task_id, task_id);
            assert_eq!(
                snapshot.results[0].outcome,
                insar_application_service::DownloadOutcome::Completed,
                "task {task_id} failed: {:?}",
                snapshot.results[0].failure
            );
            assert!(snapshot.revision >= 1);
            return snapshot.revision;
        }
        assert!(Instant::now() < deadline, "task {task_id} did not complete");
        std::thread::sleep(Duration::from_millis(25));
    }
}

#[test]
fn dem_selection_is_store_owned_and_failed_set_does_not_replace_it() {
    let temp = TempStore::new("dem-source-identity");
    let (service, _port, region_id) = service(&temp);
    let selected = StoreDemSourceIdentity {
        dataset: "COP90".to_owned(),
        provider: "OPENTOPOGRAPHY".to_owned(),
        updated_at: "2026-08-11T00:00:00Z".to_owned(),
    };
    assert_eq!(
        service
            .set_dem_source_identity(&selected)
            .expect("persist nondefault DEM selection"),
        selected
    );
    assert_eq!(
        service
            .core_restore(false)
            .expect("restore DEM selection")
            .core
            .dem_source_identity,
        Some(selected.clone())
    );

    let rejected = StoreDemSourceIdentity {
        dataset: "COP90".to_owned(),
        provider: "LOCAL_FILE".to_owned(),
        updated_at: "2026-08-11T00:00:01Z".to_owned(),
    };
    assert!(service.set_dem_source_identity(&rejected).is_err());
    assert_eq!(
        service
            .core_restore(false)
            .expect("restore last confirmed selection")
            .core
            .dem_source_identity,
        Some(selected)
    );

    let plan = service
        .plan_dem_task(&DemTaskPlanRequest {
            request_id: "dem-identity-plan".to_owned(),
            task_id: "dem-identity-task".to_owned(),
            region_id,
            resource_id: "dem.cop30.v1".to_owned(),
            capability_id: "capability.dem.cop30.aws-download.v1".to_owned(),
            dataset: "COP30".to_owned(),
            provider: "COPERNICUS_AWS".to_owned(),
            operation: DemTaskOperation::PrepareSarscape,
            input_path: temp.root.join("source-dem.tif"),
            output_root: temp.root.join("dem-output"),
            source_vertical_datum: None,
            target_vertical_datum: None,
            output_mode: None,
            created_at: "2026-08-11T00:00:02Z".to_owned(),
        })
        .expect("plan authoritative DEM task");
    service
        .create_dem_task(&DemTaskRequest {
            plan,
            concurrency: 1,
        })
        .expect("create authoritative DEM task");
    wait_for_completion(&service, "dem-identity-task");
    assert_eq!(
        service
            .core_restore(false)
            .expect("restore task-confirmed identity")
            .core
            .dem_source_identity,
        Some(StoreDemSourceIdentity {
            dataset: "COP30".to_owned(),
            provider: "COPERNICUS_AWS".to_owned(),
            updated_at: "2026-08-11T00:00:02Z".to_owned(),
        })
    );
}

#[test]
fn dem_identity_timestamp_is_canonical_for_setter_and_atomic_formal_task() {
    let temp = TempStore::new("dem-identity-timestamp");
    let (service, _port, region_id) = service(&temp);
    for (index, (input, expected)) in [
        ("2026-08-11T00:00:00.000Z", "2026-08-11T00:00:00Z"),
        ("2026-08-11T00:00:01.010Z", "2026-08-11T00:00:01.01Z"),
        ("2026-08-11T00:00:02.120Z", "2026-08-11T00:00:02.12Z"),
        ("2026-08-11T00:00:03.123Z", "2026-08-11T00:00:03.123Z"),
    ]
    .into_iter()
    .enumerate()
    {
        let confirmed = service
            .set_dem_source_identity(&StoreDemSourceIdentity {
                dataset: "COP90".to_owned(),
                provider: "OPENTOPOGRAPHY".to_owned(),
                updated_at: input.to_owned(),
            })
            .expect("canonical setter timestamp");
        assert_eq!(confirmed.updated_at, expected);

        let task_id = format!("dem-timestamp-task-{index}");
        let plan = service
            .plan_dem_task(&DemTaskPlanRequest {
                request_id: format!("dem-timestamp-plan-{index}"),
                task_id: task_id.clone(),
                region_id: region_id.clone(),
                resource_id: "dem.cop30.v1".to_owned(),
                capability_id: "capability.dem.cop30.aws-download.v1".to_owned(),
                dataset: "COP30".to_owned(),
                provider: "COPERNICUS_AWS".to_owned(),
                operation: DemTaskOperation::PrepareSarscape,
                input_path: temp.root.join(format!("source-{index}.tif")),
                output_root: temp.root.join(format!("output-{index}")),
                source_vertical_datum: None,
                target_vertical_datum: None,
                output_mode: None,
                created_at: input.to_owned(),
            })
            .expect("formal DEM plan timestamp");
        let created = service
            .create_dem_task(&DemTaskRequest {
                plan,
                concurrency: 1,
            })
            .expect("atomic formal task identity timestamp");
        assert_eq!(
            created.core.dem_source_identity.unwrap().updated_at,
            expected
        );
    }
    for invalid in [
        "2026-08-11T00:00:04.1234Z",
        "2026-08-11T00:00:04+00:00",
        "2026-08-11T00:00:04Z\n",
    ] {
        assert!(service
            .set_dem_source_identity(&StoreDemSourceIdentity {
                dataset: "COP90".to_owned(),
                provider: "OPENTOPOGRAPHY".to_owned(),
                updated_at: invalid.to_owned(),
            })
            .is_err());
    }
}

#[test]
fn standalone_bbox_dem_task_does_not_require_workspace_context() {
    let temp = TempStore::new("standalone-dem");
    let port = Arc::new(LoopbackCompatibilityPort::default());
    let service = ApplicationService::with_fixture_business_runtime(
        ApplicationServiceConfig::business_store(&temp.database)
            .with_capability_runtime_observations(
                CapabilityRuntimeObservations::default()
                    .with_build(
                        "dem-provider.copernicus-aws.v1",
                        BuildCapabilityState::Available,
                    )
                    .with_runtime(
                        "dem-provider.copernicus-aws.v1",
                        RuntimeReadinessState::Ready,
                    ),
            ),
        Arc::new(EmptySearch),
        Arc::new(LoopbackDownloadWorker),
    )
    .attach_dem_operation_port(Arc::clone(&port) as Arc<dyn DemOperationPort>);

    let snapshot = service
        .create_standalone_dem_task(&StandaloneDemTaskRequest {
            request_id: "rh1-standalone-dem-request".to_owned(),
            task_id: "rh1-standalone-dem-task".to_owned(),
            dataset: "COP30".to_owned(),
            provider: "COPERNICUS_AWS".to_owned(),
            bbox: vec![100.0, 30.0, 100.5, 30.5],
            output_root: temp.root.join("dem-output"),
            convert: true,
            concurrency: 2,
            created_at: "2026-08-11T00:00:00Z".to_owned(),
        })
        .expect("standalone DEM task creation without workspace");
    let task = snapshot
        .core
        .tasks
        .iter()
        .find(|task| task.task_id == "rh1-standalone-dem-task")
        .expect("standalone task is in the bounded Store projection");
    assert_eq!(
        task.operation
            .as_ref()
            .map(|operation| operation.region_id.as_str()),
        Some("standalone-dem")
    );
    wait_for_completion(&service, "rh1-standalone-dem-task");
    let payloads = port.payloads.lock().unwrap().clone();
    assert_eq!(
        payloads
            .first()
            .and_then(|payload| payload.get("bbox"))
            .and_then(|bbox| bbox.get("west"))
            .and_then(Value::as_f64),
        Some(100.0)
    );
}

#[test]
fn standalone_dem_download_persists_canonical_identity_timestamp() {
    for (index, (input, expected)) in [
        ("2026-08-11T00:00:00.000Z", "2026-08-11T00:00:00Z"),
        ("2026-08-11T00:00:01.010Z", "2026-08-11T00:00:01.01Z"),
        ("2026-08-11T00:00:02.120Z", "2026-08-11T00:00:02.12Z"),
        ("2026-08-11T00:00:03.123Z", "2026-08-11T00:00:03.123Z"),
    ]
    .into_iter()
    .enumerate()
    {
        let temp = TempStore::new(&format!("standalone-dem-timestamp-{index}"));
        let port = Arc::new(LoopbackCompatibilityPort::default());
        let service = ApplicationService::with_fixture_business_runtime(
            ApplicationServiceConfig::business_store(&temp.database)
                .with_capability_runtime_observations(
                    CapabilityRuntimeObservations::default()
                        .with_build(
                            "dem-provider.copernicus-aws.v1",
                            BuildCapabilityState::Available,
                        )
                        .with_runtime(
                            "dem-provider.copernicus-aws.v1",
                            RuntimeReadinessState::Ready,
                        ),
                ),
            Arc::new(EmptySearch),
            Arc::new(LoopbackDownloadWorker),
        )
        .attach_dem_operation_port(Arc::clone(&port) as Arc<dyn DemOperationPort>);
        let created = service
            .create_standalone_dem_task(&StandaloneDemTaskRequest {
                request_id: format!("standalone-timestamp-request-{index}"),
                task_id: format!("standalone-timestamp-task-{index}"),
                dataset: "COP30".to_owned(),
                provider: "COPERNICUS_AWS".to_owned(),
                bbox: vec![100.0, 30.0, 100.5, 30.5],
                output_root: temp.root.join(format!("output-{index}")),
                convert: true,
                concurrency: 1,
                created_at: input.to_owned(),
            })
            .expect("standalone DEM identity timestamp");
        assert_eq!(
            created.core.dem_source_identity.unwrap().updated_at,
            expected
        );
    }
}

#[test]
fn dem_plan_task_completion_and_conversion_use_one_shared_lifecycle() {
    let temp = TempStore::new("dem");
    let (service, port, region_id) = service(&temp);
    let plan = service
        .plan_dem_task(&DemTaskPlanRequest {
            request_id: "dem-plan".to_owned(),
            task_id: "dem-task-rh1".to_owned(),
            region_id: region_id.clone(),
            resource_id: "dem.cop30.v1".to_owned(),
            capability_id: "capability.dem.cop30.aws-download.v1".to_owned(),
            dataset: "COP30".to_owned(),
            provider: "COPERNICUS_AWS".to_owned(),
            operation: DemTaskOperation::PrepareSarscape,
            input_path: temp.root.join("source-dem.tif"),
            output_root: temp.root.join("dem-output"),
            source_vertical_datum: None,
            target_vertical_datum: None,
            output_mode: None,
            created_at: "2026-08-11T00:00:00Z".to_owned(),
        })
        .unwrap();
    assert_eq!(plan.task_id, "dem-task-rh1");
    let created = service
        .create_dem_task(&DemTaskRequest {
            plan,
            concurrency: 1,
        })
        .unwrap();
    assert!(created
        .core
        .tasks
        .iter()
        .any(|task| task.task_id == "dem-task-rh1"));
    wait_for_completion(&service, "dem-task-rh1");
    let source_revision = service
        .core_restore(false)
        .expect("source task restore")
        .core
        .tasks
        .iter()
        .find(|task| task.task_id == "dem-task-rh1")
        .expect("completed DEM source is active before archive")
        .revision;
    let source_request = StoreDemConversionSourceRequest {
        region_id: region_id.clone(),
        dataset: "COP30".to_owned(),
        provider: "COPERNICUS_AWS".to_owned(),
        resource_id: "dem.cop30.v1".to_owned(),
        capability_id: "capability.dem.cop30.aws-download.v1".to_owned(),
    };
    let current_source = service
        .resolve_dem_conversion_source(&source_request)
        .expect("resolve current DEM source")
        .expect("current DEM source exists");
    assert_eq!(current_source.task_id, "dem-task-rh1");
    assert_eq!(current_source.input_path, temp.root.join("source-dem.tif"));
    assert_eq!(current_source.output_root, temp.root.join("dem-output"));
    service
        .archive_download_task(&StoreArchiveTaskRequest {
            task_id: "dem-task-rh1".to_owned(),
            expected_revision: source_revision,
            archived_at: "2026-08-11T00:00:03Z".to_owned(),
        })
        .expect("archive completed DEM source");
    assert!(service
        .core_restore(false)
        .expect("active-only restore")
        .core
        .tasks
        .is_empty());
    let resolved = service
        .resolve_dem_conversion_source(&source_request)
        .expect("resolve archived DEM source")
        .expect("archived DEM source exists");
    assert_eq!(resolved.task_id, "dem-task-rh1");
    assert_eq!(resolved.revision, source_revision);
    assert_eq!(resolved.region_id, region_id);
    assert_eq!(resolved.dataset, "COP30");
    assert_eq!(resolved.provider, "COPERNICUS_AWS");
    assert_eq!(resolved.resource_id, "dem.cop30.v1");
    assert_eq!(
        resolved.capability_id,
        "capability.dem.cop30.aws-download.v1"
    );
    assert_eq!(resolved.input_path, temp.root.join("source-dem.tif"));
    assert_eq!(resolved.output_root, temp.root.join("dem-output"));

    let valid_handoff = DemConversionRequest {
        request_id: "dem-convert".to_owned(),
        source_task_id: resolved.task_id,
        conversion_task_id: "dem-task-rh1-conversion".to_owned(),
        region_id: region_id.clone(),
        resource_id: "dem.cop30.v1".to_owned(),
        capability_id: "capability.dem.cop30.aws-download.v1".to_owned(),
        dataset: "COP30".to_owned(),
        provider: "COPERNICUS_AWS".to_owned(),
        input_path: temp.root.join("source-dem.tif"),
        output_root: temp.root.join("dem-output"),
        source_vertical_datum: None,
        target_vertical_datum: None,
        output_mode: None,
        created_at: "2026-08-11T00:01:00Z".to_owned(),
        concurrency: 1,
    };
    for (suffix, mutate) in [
        ("dataset", 0_u8),
        ("provider", 1),
        ("input", 2),
        ("output", 3),
        ("region", 4),
    ] {
        let mut invalid = valid_handoff.clone();
        invalid.request_id = format!("invalid-{suffix}");
        invalid.conversion_task_id = format!("invalid-conversion-{suffix}");
        match mutate {
            0 => invalid.dataset = "cop30".to_owned(),
            1 => invalid.provider = "copernicus_aws".to_owned(),
            2 => invalid.input_path = temp.root.join("wrong-input.tif"),
            3 => invalid.output_root = temp.root.join("wrong-output"),
            4 => invalid.region_id = "wrong-region".to_owned(),
            _ => unreachable!(),
        }
        assert!(
            service.handoff_dem_conversion(&invalid).is_err(),
            "direct handoff must reject source {suffix} mismatch"
        );
    }

    let conversion = service.handoff_dem_conversion(&valid_handoff).unwrap();
    assert!(conversion
        .core
        .tasks
        .iter()
        .any(|task| task.task_id == "dem-task-rh1-conversion"));
    wait_for_completion(&service, "dem-task-rh1-conversion");
    let operations = port.operations.lock().unwrap().clone();
    assert_eq!(operations, ["dem_prepare_sarscape", "dem_convert"]);
    let payloads = port.payloads.lock().unwrap().clone();
    assert_eq!(payloads[0]["resource_id"], "dem.cop30.v1");
    assert_eq!(
        payloads[0]["capability_id"],
        "capability.dem.cop30.aws-download.v1"
    );
    assert_eq!(payloads[0]["dataset"], "COP30");
    assert_eq!(
        payloads[0]["output_dir"],
        temp.root.join("dem-output").to_string_lossy().as_ref()
    );
    assert_eq!(payloads[0]["bbox"]["west"], 100.0);
    assert_eq!(payloads[0]["bbox"]["east"], 101.0);
    assert_eq!(payloads[1]["source_task_id"], "dem-task-rh1");
    assert_eq!(payloads[1]["execute"], true);
    service.shutdown().unwrap();
}

#[test]
fn direct_handoff_rejects_a_completed_dem_task_with_the_wrong_operation() {
    let temp = TempStore::new("dem-wrong-source-operation");
    let (service, _port, region_id) = service(&temp);
    let plan = service
        .plan_dem_task(&DemTaskPlanRequest {
            request_id: "wrong-op-plan".to_owned(),
            task_id: "wrong-op-source".to_owned(),
            region_id: region_id.clone(),
            resource_id: "dem.user-local.v1".to_owned(),
            capability_id: "capability.dem.user-local.convert.v1".to_owned(),
            dataset: "USER_LOCAL".to_owned(),
            provider: "LOCAL_FILE".to_owned(),
            operation: DemTaskOperation::Convert,
            input_path: temp.root.join("wrong-op-input.tif"),
            output_root: temp.root.join("wrong-op-output"),
            source_vertical_datum: Some("EGM96".to_owned()),
            target_vertical_datum: Some("WGS84_ELLIPSOID".to_owned()),
            output_mode: Some("sarscape".to_owned()),
            created_at: "2026-08-11T00:00:00Z".to_owned(),
        })
        .unwrap();
    service
        .create_dem_task(&DemTaskRequest {
            plan,
            concurrency: 1,
        })
        .unwrap();
    wait_for_completion(&service, "wrong-op-source");

    assert!(service
        .handoff_dem_conversion(&DemConversionRequest {
            request_id: "wrong-op-handoff".to_owned(),
            source_task_id: "wrong-op-source".to_owned(),
            conversion_task_id: "wrong-op-conversion".to_owned(),
            region_id,
            resource_id: "dem.user-local.v1".to_owned(),
            capability_id: "capability.dem.user-local.convert.v1".to_owned(),
            dataset: "USER_LOCAL".to_owned(),
            provider: "LOCAL_FILE".to_owned(),
            input_path: temp.root.join("wrong-op-input.tif"),
            output_root: temp.root.join("wrong-op-output"),
            source_vertical_datum: Some("EGM96".to_owned()),
            target_vertical_datum: Some("WGS84_ELLIPSOID".to_owned()),
            output_mode: Some("sarscape".to_owned()),
            created_at: "2026-08-11T00:01:00Z".to_owned(),
            concurrency: 1,
        })
        .is_err());
    service.shutdown().unwrap();
}

#[test]
fn local_dem_conversion_is_a_registry_bound_formal_task() {
    let temp = TempStore::new("local-dem");
    let (service, port, region_id) = service(&temp);
    let plan = service
        .plan_dem_task(&DemTaskPlanRequest {
            request_id: "local-dem-plan".to_owned(),
            task_id: "local-dem-task-rh1".to_owned(),
            region_id: region_id.clone(),
            resource_id: "dem.user-local.v1".to_owned(),
            capability_id: "capability.dem.user-local.convert.v1".to_owned(),
            dataset: "USER_LOCAL".to_owned(),
            provider: "LOCAL_FILE".to_owned(),
            operation: DemTaskOperation::Convert,
            input_path: temp.root.join("uploaded_dem.tif"),
            output_root: temp.root.join("local-dem-output"),
            source_vertical_datum: Some("EGM96".to_owned()),
            target_vertical_datum: Some("WGS84_ELLIPSOID".to_owned()),
            output_mode: Some("ellipsoid".to_owned()),
            created_at: "2026-08-11T00:03:00Z".to_owned(),
        })
        .unwrap();
    service
        .create_dem_task(&DemTaskRequest {
            plan,
            concurrency: 1,
        })
        .unwrap();
    wait_for_completion(&service, "local-dem-task-rh1");
    assert_eq!(port.operations.lock().unwrap().as_slice(), ["dem_convert"]);
    let payload = &port.payloads.lock().unwrap()[0];
    assert_eq!(payload["resource_id"], "dem.user-local.v1");
    assert_eq!(
        payload["capability_id"],
        "capability.dem.user-local.convert.v1"
    );
    assert_eq!(payload["dataset"], "USER_LOCAL");
    assert_eq!(payload["provider"], "LOCAL_FILE");
    assert_eq!(payload["execute"], true);
    assert_eq!(payload["source_vertical_datum"], "EGM96");
    assert_eq!(payload["output_mode"], "ellipsoid");
    service.shutdown().unwrap();
}

#[test]
fn orbit_association_is_frozen_into_the_shared_task_route() {
    let temp = TempStore::new("orbit");
    let (service, port, region_id) = service(&temp);
    let association = FrozenOrbitAssociationV1 {
        association_version: 1,
        source_item_id: "S1A_SCENE_RH1".to_owned(),
        orbit_transfer_item_id: "orbit-poeorb-rh1".to_owned(),
        requested_orbit_type: "POEORB".to_owned(),
        source_platform: OrbitPlatform::S1A,
        source_acquisition_start: 1_700_000_000,
        source_acquisition_stop: 1_700_001_000,
        matched_orbit_metadata: OrbitFileMetadata {
            file_name: "S1A_OPER_AUX_POEORB.EOF".to_owned(),
            platform: OrbitPlatform::S1A,
            orbit_type: OrbitProductType::POEORB,
            creation_unix_seconds: 1_700_002_000,
            validity_start_unix_seconds: 1_699_999_000,
            validity_stop_unix_seconds: 1_700_010_000,
        },
        selection_policy_id: "sentinel1.poeorb.frozen.v1".to_owned(),
    };
    let snapshot = service
        .create_orbit_task_legacy(&OrbitTaskRequest {
            request_id: "orbit-task-request".to_owned(),
            task_id: "orbit-task-rh1".to_owned(),
            region_id,
            association,
            output_root: temp.root.join("orbit-output"),
            created_at: "2026-08-11T00:02:00Z".to_owned(),
            concurrency: 1,
        })
        .unwrap();
    let task = snapshot
        .restore
        .tasks
        .iter()
        .find(|task| task.task_id == "orbit-task-rh1")
        .unwrap();
    assert_eq!(task.operation.as_ref().unwrap().kind, "orbit");
    assert!(task.orbit_association.is_some());
    wait_for_completion(&service, "orbit-task-rh1");
    assert!(port.operations.lock().unwrap().is_empty());
    service.shutdown().unwrap();
}

#[test]
fn standalone_dem_creation_fails_closed_on_corrupt_network_settings() {
    let temp = TempStore::new("standalone-dem-corrupt-network");
    std::fs::write(
        temp.database.with_file_name("network-settings.json"),
        b"{invalid json",
    )
    .unwrap();
    let port = Arc::new(LoopbackCompatibilityPort::default());
    let service = ApplicationService::with_fixture_business_runtime(
        ApplicationServiceConfig::business_store(&temp.database)
            .with_capability_runtime_observations(
                CapabilityRuntimeObservations::default()
                    .with_build(
                        "dem-provider.copernicus-aws.v1",
                        BuildCapabilityState::Available,
                    )
                    .with_runtime(
                        "dem-provider.copernicus-aws.v1",
                        RuntimeReadinessState::Ready,
                    ),
            ),
        Arc::new(EmptySearch),
        Arc::new(LoopbackDownloadWorker),
    )
    .attach_dem_operation_port(port as Arc<dyn DemOperationPort>);

    let err = service
        .create_standalone_dem_task(&StandaloneDemTaskRequest {
            request_id: "rh1-corrupt-network-request".to_owned(),
            task_id: "rh1-corrupt-network-task".to_owned(),
            dataset: "COP30".to_owned(),
            provider: "COPERNICUS_AWS".to_owned(),
            bbox: vec![100.0, 30.0, 100.5, 30.5],
            output_root: temp.root.join("dem-output"),
            convert: true,
            concurrency: 1,
            created_at: "2026-08-11T00:00:00Z".to_owned(),
        })
        .expect_err(
            "corrupt network-settings.json must fail closed rather than silently falling back",
        );
    assert_eq!(err.code(), "V3_APPLICATION_INVALID_REQUEST");

    let aoi_geojson = serde_json::json!({
        "type": "Polygon",
        "coordinates": [[[100.0, 30.0], [100.5, 30.0], [100.5, 30.5], [100.0, 30.5], [100.0, 30.0]]]
    })
    .as_object()
    .unwrap()
    .clone();
    let aoi_err = service
        .create_standalone_dem_aoi_task(&StandaloneDemAoiTaskRequest {
            request_id: "rh1-corrupt-network-aoi-request".to_owned(),
            task_id: "rh1-corrupt-network-aoi-task".to_owned(),
            dataset: "COP30".to_owned(),
            provider: "COPERNICUS_AWS".to_owned(),
            aoi_geojson,
            aoi_name: "Test AOI".to_owned(),
            output_root: temp.root.join("dem-output"),
            convert: true,
            concurrency: 1,
            created_at: "2026-08-11T00:00:00Z".to_owned(),
        })
        .expect_err("standalone DEM AOI creation must also fail closed on corrupt settings");
    assert_eq!(aoi_err.code(), "V3_APPLICATION_INVALID_REQUEST");
    service.shutdown().unwrap();
}

#[test]
fn standalone_orbit_task_creates_without_legacy_workspace_and_dedupes_scenes() {
    let temp = TempStore::new("standalone-orbit-no-workspace");
    let port = Arc::new(LoopbackCompatibilityPort::default());
    let service = ApplicationService::with_fixture_business_runtime(
        ApplicationServiceConfig::business_store(&temp.database)
            .with_capability_runtime_observations(
                CapabilityRuntimeObservations::default().with_runtime(
                    "orbit-download.asf-sentinel-1.v1",
                    RuntimeReadinessState::Ready,
                ),
            ),
        Arc::new(EmptySearch),
        Arc::new(LoopbackDownloadWorker),
    )
    .attach_dem_operation_port(port);

    let scene_a = "S1A_IW_SLC__1SDV_20240312T100000_20240312T100027_052946_066912_ABCD";
    let scene_b = "S1B_IW_SLC__1SDV_20240313T100000_20240313T100027_041946_055912_EF01";
    let output_root = temp.root.join("orbit-standalone-output");

    let snapshot = service
        .create_standalone_orbit_task(&insar_application_service::StandaloneOrbitTaskRequest {
            request_id: "standalone-orbit-req-1".to_owned(),
            task_id: "standalone-orbit-task-1".to_owned(),
            scenes: vec![
                insar_application_service::StandaloneOrbitSceneInput {
                    scene_id: scene_a.to_owned(),
                    acquisition_datetime: Some("2024-03-12T10:00:00Z".to_owned()),
                },
                insar_application_service::StandaloneOrbitSceneInput {
                    scene_id: scene_a.to_owned(),
                    acquisition_datetime: None,
                },
                insar_application_service::StandaloneOrbitSceneInput {
                    scene_id: scene_b.to_owned(),
                    acquisition_datetime: Some("2024-03-13T10:00:00Z".to_owned()),
                },
            ],
            output_root: output_root.clone(),
            use_orbit_subdir: true,
            created_at: "2026-08-11T00:02:00Z".to_owned(),
        })
        .expect("standalone orbit should succeed without workspace/project/region/AOI");

    let task = snapshot
        .core
        .tasks
        .iter()
        .find(|task| task.task_id == "standalone-orbit-task-1")
        .expect("standalone orbit task in core snapshot");
    assert_eq!(task.provider_id, "sentinel1.orbit");
    assert_eq!(task.concurrency, Some(1));
    assert_eq!(task.selected_count, 2);
    assert_eq!(task.auxiliary_kind.as_deref(), Some("AUX_POEORB"));
    assert!(snapshot.core.workbench.is_none());

    let readonly = service.snapshot(true).unwrap();
    let stored = readonly
        .restore
        .tasks
        .iter()
        .find(|task| task.task_id == "standalone-orbit-task-1")
        .expect("standalone orbit task in readonly snapshot");
    assert_eq!(
        stored.layout,
        insar_core::DownloadLayout::SentinelOrbitAuxPoeorb
    );
    assert_eq!(stored.initial_selected_item_ids, vec![scene_a, scene_b]);
    assert_eq!(
        stored
            .operation
            .as_ref()
            .map(|op| op.approved_worker_operation.as_str()),
        Some("orbit_download")
    );

    let flat_snapshot = service
        .create_standalone_orbit_task(&insar_application_service::StandaloneOrbitTaskRequest {
            request_id: "standalone-orbit-req-2".to_owned(),
            task_id: "standalone-orbit-task-2".to_owned(),
            scenes: vec![insar_application_service::StandaloneOrbitSceneInput {
                scene_id: scene_a.to_owned(),
                acquisition_datetime: None,
            }],
            output_root,
            use_orbit_subdir: false,
            created_at: "2026-08-11T00:03:00Z".to_owned(),
        })
        .expect("flat standalone orbit should succeed");
    assert!(flat_snapshot
        .core
        .tasks
        .iter()
        .any(|task| task.task_id == "standalone-orbit-task-2"));
    let flat_readonly = service.snapshot(true).unwrap();
    let flat_task = flat_readonly
        .restore
        .tasks
        .iter()
        .find(|task| task.task_id == "standalone-orbit-task-2")
        .unwrap();
    assert_eq!(flat_task.layout, insar_core::DownloadLayout::Direct);

    let bad_scene = service
        .create_standalone_orbit_task(&insar_application_service::StandaloneOrbitTaskRequest {
            request_id: "standalone-orbit-req-bad".to_owned(),
            task_id: "standalone-orbit-task-bad".to_owned(),
            scenes: vec![insar_application_service::StandaloneOrbitSceneInput {
                scene_id: "NOT_A_VALID_S1_SCENE".to_owned(),
                acquisition_datetime: None,
            }],
            output_root: temp.root.join("orbit-bad"),
            use_orbit_subdir: true,
            created_at: "2026-08-11T00:04:00Z".to_owned(),
        })
        .expect_err("invalid S1 scene must be rejected");
    assert_eq!(bad_scene.code(), "V3_APPLICATION_INVALID_REQUEST");
    service.shutdown().unwrap();
}

#[test]
fn standalone_orbit_download_strips_asf_suffix_and_skips_non_s1_scenes() {
    let temp = TempStore::new("standalone_orbit_asf_suffix");
    let port = Arc::new(LoopbackCompatibilityPort::default());
    let service = ApplicationService::with_fixture_business_runtime(
        ApplicationServiceConfig::business_store(&temp.database)
            .with_capability_runtime_observations(
                CapabilityRuntimeObservations::default().with_runtime(
                    "orbit-download.asf-sentinel-1.v1",
                    RuntimeReadinessState::Ready,
                ),
            ),
        Arc::new(EmptySearch),
        Arc::new(LoopbackDownloadWorker),
    )
    .attach_dem_operation_port(port);

    let scene_slc_suffixed =
        "S1A_IW_SLC__1SDV_20240101T000000_20240101T000027_051910_0645A3_1234-SLC";
    let scene_grd_suffixed =
        "S1B_IW_GRDH_1SDV_20240102T000000_20240102T000025_040000_050001_ABCD-GRD_HD";
    let non_s1_scene = "NISAR_L1_PR_RSLC_001";

    let snapshot = service
        .create_standalone_orbit_task(&insar_application_service::StandaloneOrbitTaskRequest {
            request_id: "standalone-orbit-req-mixed".to_owned(),
            task_id: "standalone-orbit-task-mixed".to_owned(),
            scenes: vec![
                insar_application_service::StandaloneOrbitSceneInput {
                    scene_id: scene_slc_suffixed.to_owned(),
                    acquisition_datetime: None,
                },
                insar_application_service::StandaloneOrbitSceneInput {
                    scene_id: non_s1_scene.to_owned(),
                    acquisition_datetime: Some("2025-10-01T00:00:00Z".to_owned()),
                },
                insar_application_service::StandaloneOrbitSceneInput {
                    scene_id: scene_grd_suffixed.to_owned(),
                    acquisition_datetime: None,
                },
            ],
            output_root: temp.root.join("orbit-mixed"),
            use_orbit_subdir: true,
            created_at: "2026-08-11T00:05:00Z".to_owned(),
        })
        .expect("mixed batch should keep valid Sentinel-1 scenes after stripping ASF suffixes");
    assert!(snapshot
        .core
        .tasks
        .iter()
        .any(|task| task.task_id == "standalone-orbit-task-mixed"));

    let readonly = service.snapshot(true).unwrap();
    let stored = readonly
        .restore
        .tasks
        .iter()
        .find(|task| task.task_id == "standalone-orbit-task-mixed")
        .unwrap();
    assert_eq!(
        stored.initial_selected_item_ids,
        vec![
            "S1A_IW_SLC__1SDV_20240101T000000_20240101T000027_051910_0645A3_1234",
            "S1B_IW_GRDH_1SDV_20240102T000000_20240102T000025_040000_050001_ABCD",
        ]
    );
    service.shutdown().unwrap();
}

#[test]
fn standalone_orbit_download_strips_canonical_provider_item_prefix_including_s1d() {
    let temp = TempStore::new("standalone_orbit_provider_prefix");
    let port = Arc::new(LoopbackCompatibilityPort::default());
    let service = ApplicationService::with_fixture_business_runtime(
        ApplicationServiceConfig::business_store(&temp.database)
            .with_capability_runtime_observations(
                CapabilityRuntimeObservations::default().with_runtime(
                    "orbit-download.asf-sentinel-1.v1",
                    RuntimeReadinessState::Ready,
                ),
            ),
        Arc::new(EmptySearch),
        Arc::new(LoopbackDownloadWorker),
    )
    .attach_dem_operation_port(port);

    let canonical_s1d =
        "PROVIDER::asf.catalog::COLLECTION::SENTINEL-1::ITEM::S1D_IW_SLC__1SDV_20260825T101857_20260825T101925_004281_007E0D_FB72-SLC";
    service
        .create_standalone_orbit_task(&insar_application_service::StandaloneOrbitTaskRequest {
            request_id: "standalone-orbit-req-s1d".to_owned(),
            task_id: "standalone-orbit-task-s1d".to_owned(),
            scenes: vec![insar_application_service::StandaloneOrbitSceneInput {
                scene_id: canonical_s1d.to_owned(),
                acquisition_datetime: Some("2026-08-25T10:18:57Z".to_owned()),
            }],
            output_root: temp.root.join("orbit-s1d"),
            use_orbit_subdir: false,
            created_at: "2026-08-26T00:05:00Z".to_owned(),
        })
        .expect("canonical PROVIDER::...::ITEM:: S1D scene_id should succeed");

    let readonly = service.snapshot(true).unwrap();
    let stored = readonly
        .restore
        .tasks
        .iter()
        .find(|task| task.task_id == "standalone-orbit-task-s1d")
        .unwrap();
    assert_eq!(
        stored.initial_selected_item_ids,
        vec!["S1D_IW_SLC__1SDV_20260825T101857_20260825T101925_004281_007E0D_FB72"]
    );
    service.shutdown().unwrap();
}
