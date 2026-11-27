//! Opt-in native DEM acceptance: AWS is anonymous; OpenTopography requires a key.
//! Online download and existing-Store mutation cases require separate authorization.
use eostage_desktop::dem_live_acceptance_support::state;
use insar_application_service::{
    DemTaskOperation, StandaloneDemAoiTaskRequest, StandaloneDemRequest,
};
use insar_core::TaskLifecycleStatus;
use serde_json::json;
use std::{
    fs,
    path::PathBuf,
    thread,
    time::{Duration, Instant},
};

fn detailed_acceptance_aoi() -> serde_json::Value {
    let ring = |west: f64, south: f64, east: f64, north: f64| {
        let corners = [
            [west, south],
            [east, south],
            [east, north],
            [west, north],
            [west, south],
        ];
        let mut points = Vec::new();
        for edge in corners.windows(2) {
            for step in 0..200 {
                let t = f64::from(step) / 200.0;
                points.push(vec![
                    edge[0][0] + (edge[1][0] - edge[0][0]) * t,
                    edge[0][1] + (edge[1][1] - edge[0][1]) * t,
                ]);
            }
        }
        points.push(vec![west, south]);
        points
    };
    json!({"type":"MultiPolygon", "coordinates":[
        [ring(100.0,30.0,100.01,30.01), ring(100.002,30.002,100.008,30.008)],
        [ring(100.003,30.003,100.007,30.007)]
    ]})
}

#[test]
#[ignore = "requires an explicitly authorized provider request and fresh evidence root"]
fn online_download_and_convert_native_outputs() {
    assert_eq!(
        std::env::var("INSAR_DEM_LIVE_ACCEPTANCE").as_deref(),
        Ok("1")
    );
    let dataset = std::env::var("INSAR_COMPLETION_DEM_DATASET").unwrap_or_else(|_| "COP90".into());
    assert!(matches!(dataset.as_str(), "COP30" | "COP90"));
    let provider =
        std::env::var("INSAR_COMPLETION_DEM_PROVIDER").unwrap_or_else(|_| "COPERNICUS_AWS".into());
    assert!(matches!(
        provider.as_str(),
        "COPERNICUS_AWS" | "OPENTOPOGRAPHY"
    ));
    if provider == "OPENTOPOGRAPHY" {
        assert_eq!(
            insar_download::desktop_opentopography_status(),
            "set",
            "OpenTopography saved key is required; do not print its value"
        );
    }
    let convert = std::env::var("INSAR_COMPLETION_DOWNLOAD_ONLY").as_deref() != Ok("1");
    let root =
        PathBuf::from(std::env::var_os("INSAR_COMPLETION_CONVERT_ROOT").expect("fresh root"));
    let runtime =
        PathBuf::from(std::env::var_os("INSAR_COMPLETION_GIS_ROOT").expect("product runtime"));
    assert!(root.is_absolute() && !root.exists());
    fs::create_dir_all(root.join("output")).unwrap();
    let service = state(root.join("tasks.sqlite3"), runtime.clone()).unwrap();
    let output = root.join("output");
    let task_id = "completion-online-convert";
    let geometry = detailed_acceptance_aoi();
    service
        .set_working_aoi(
            &insar_application_service::ApplicationSetWorkingAoiRequest {
                source_kind: "manual_polygon".into(),
                geometry: geometry.as_object().unwrap().clone(),
                bbox: vec![100.0, 30.0, 100.01, 30.01],
                crs: "EPSG:4326".into(),
                display_name: "fixed acceptance AOI".into(),
            },
        )
        .unwrap();
    service
        .create_standalone_dem_task(&StandaloneDemAoiTaskRequest {
            request_id: "completion-online-create".into(),
            task_id: task_id.into(),
            dataset: dataset.clone(),
            provider: provider.clone(),
            aoi_geojson: geometry.as_object().unwrap().clone(),
            aoi_name: "fixed acceptance AOI".into(),
            output_root: output.clone(),
            convert,
            concurrency: 1,
            created_at: "2026-09-07T00:00:00Z".into(),
        })
        .unwrap();
    let started = Instant::now();
    let mut peak_rate = 0.0_f64;
    let mut observations = Vec::new();
    let mut received_before_completion = false;
    let mut previous_received = 0;
    let task = loop {
        let snapshot = service.core_restore(true).unwrap();
        let task = snapshot
            .core
            .tasks
            .iter()
            .find(|task| task.task_id == task_id)
            .unwrap();
        peak_rate = peak_rate.max(task.bytes_per_second);
        if let Some(received) = task.network_bytes_received {
            assert!(
                received >= previous_received,
                "source byte progress must not regress"
            );
            previous_received = received;
            received_before_completion |=
                task.status == TaskLifecycleStatus::Running && received > 0;
            observations.push(json!({"elapsed_ms": started.elapsed().as_millis(),
                "network_bytes_received": received, "status": task.status,
                "bytes_per_second": task.bytes_per_second}));
        }
        fs::write(
            root.join("task-state.json"),
            serde_json::to_vec_pretty(task).unwrap(),
        )
        .unwrap();
        if task.status == TaskLifecycleStatus::Completed {
            break task.clone();
        }
        assert!(
            !matches!(
                task.status,
                TaskLifecycleStatus::Failed
                    | TaskLifecycleStatus::Cancelled
                    | TaskLifecycleStatus::Interrupted
            ),
            "native download+convert entered terminal failure; task-state.json retained"
        );
        assert!(
            started.elapsed() < Duration::from_secs(600),
            "bounded acceptance deadline exceeded"
        );
        thread::sleep(Duration::from_millis(200));
    };
    assert_eq!(task.completed_count, 1);
    assert!(task.network_bytes_received.is_some_and(|bytes| bytes > 0));
    if provider == "COPERNICUS_AWS" {
        assert!(
            received_before_completion,
            "real network bytes must be visible while downloading, not only at completion"
        );
        assert!(
            task.network_bytes_received.unwrap() > task.transferred_bytes,
            "full source tile bytes must remain distinct from the tiny cropped raster"
        );
    }
    fs::write(
        root.join("progress-observations.json"),
        serde_json::to_vec_pretty(&observations).unwrap(),
    )
    .unwrap();
    fs::write(
        root.join("transfer-observation.json"),
        serde_json::to_vec_pretty(&json!({
            "dataset": dataset, "provider": provider, "convert": convert, "peak_observed_bytes_per_second": peak_rate,
            "transfer_active_millis": task.transfer_active_millis,
            "final_bytes_per_second": task.bytes_per_second,
            "boundary": "production native service; not Workbench/Tauri UI acceptance"
        }))
        .unwrap(),
    )
    .unwrap();
    // AWS downloads full source tiles here. OpenTopography returns a tiny
    // cropped response whose buffered body can finish within one millisecond;
    // the shared registry floors milliseconds and samples rate once a second.
    if provider == "COPERNICUS_AWS" {
        assert!(
            task.transfer_active_millis > 0,
            "tile HTTP body time must reach Store"
        );
    }
    assert!(task.transferred_bytes > 0);
    assert_eq!(
        task.bytes_per_second, 0.0,
        "completed task must not retain an active rate"
    );
    let mut reports = Vec::new();
    let suffixes = if convert {
        vec!["_dem.tif", "_ellipsoid.tif", "_dem"]
    } else {
        vec!["_dem.tif"]
    };
    for suffix in suffixes {
        let name = format!("{dataset}{suffix}");
        let path = output.join(&name);
        assert!(path.is_file(), "missing native output {name}");
        let inspection = service
            .standalone_dem(&StandaloneDemRequest {
                request_id: format!("inspect-{}", reports.len()),
                dataset: dataset.clone(),
                provider: provider.clone(),
                operation: DemTaskOperation::Inspect,
                input_path: path,
                output_root: output.clone(),
                bbox: None,
                source_vertical_datum: Some("EGM2008".into()),
                target_vertical_datum: Some("WGS84_ELLIPSOID".into()),
                output_mode: Some("sarscape".into()),
            })
            .unwrap();
        assert_eq!(inspection["result"]["readable"], true);
        reports.push(inspection);
    }
    for suffix in if convert { vec!["hdr", "sml"] } else { vec![] } {
        assert!(
            fs::metadata(output.join(format!("{dataset}_dem.{suffix}")))
                .unwrap()
                .len()
                > 0
        );
    }
    fs::write(
        root.join("conversion-results.json"),
        serde_json::to_vec_pretty(&reports).unwrap(),
    )
    .unwrap();
    if !convert {
        assert!(!output.join(format!("{dataset}_ellipsoid.tif")).exists());
        assert!(!output.join(format!("{dataset}_dem")).exists());
    }
    service
        .archive_download_task(&serde_json::from_value(json!({
            "task_id": task_id, "expected_revision": task.revision, "archived_at": task.updated_at,
        })).unwrap())
        .unwrap();
    assert!(service.core_restore(false).unwrap().core.tasks.is_empty());
    service.shutdown().unwrap();
    drop(service);
    let reopened = state(root.join("tasks.sqlite3"), runtime).unwrap();
    let history = reopened
        .task_history_page(
            &serde_json::from_value(json!({
                "page_size": 40, "after_archived_at_epoch_ms": null, "after_task_id": null,
            }))
            .unwrap(),
        )
        .unwrap();
    assert_eq!(history.items.len(), 1);
    assert_eq!(history.items[0].task.task_id, task_id);
    assert_eq!(history.items[0].task.status, TaskLifecycleStatus::Completed);
    assert_eq!(
        history.items[0].task.operation.as_ref().unwrap().kind,
        "dem"
    );
    fs::write(
        root.join("history-after-restart.json"),
        serde_json::to_vec_pretty(&history).unwrap(),
    )
    .unwrap();
    assert!(output.join(format!("{dataset}_dem.tif")).is_file());
    reopened.shutdown().unwrap();
}

#[test]
#[ignore = "mutates an isolated COP30 acceptance Store by archiving completed tasks; no download"]
fn existing_dem_outputs_and_archive_survive_restart() {
    let root =
        PathBuf::from(std::env::var_os("INSAR_COMPLETION_CONVERT_ROOT").expect("existing root"));
    let runtime =
        PathBuf::from(std::env::var_os("INSAR_COMPLETION_GIS_ROOT").expect("product runtime"));
    assert!(root.join("tasks.sqlite3").is_file());
    let reopened = state(root.join("tasks.sqlite3"), runtime).unwrap();
    let current = reopened.core_restore(false).unwrap();
    for task in current.core.tasks {
        assert_eq!(task.task_id, "completion-online-convert");
        assert_eq!(task.status, TaskLifecycleStatus::Completed);
        reopened.archive_download_task(&serde_json::from_value(json!({
            "task_id": task.task_id, "expected_revision": task.revision, "archived_at": task.updated_at,
        })).unwrap()).unwrap();
    }
    assert!(reopened.core_restore(false).unwrap().core.tasks.is_empty());
    let history = reopened
        .task_history_page(
            &serde_json::from_value(json!({
                "page_size": 40, "after_archived_at_epoch_ms": null, "after_task_id": null,
            }))
            .unwrap(),
        )
        .unwrap();
    assert_eq!(history.items.len(), 1);
    assert_eq!(history.items[0].task.task_id, "completion-online-convert");
    assert_eq!(history.items[0].task.status, TaskLifecycleStatus::Completed);
    assert_eq!(
        history.items[0].task.operation.as_ref().unwrap().kind,
        "dem"
    );
    fs::write(
        root.join("history-after-restart.json"),
        serde_json::to_vec_pretty(&history).unwrap(),
    )
    .unwrap();
    assert!(root.join("output/COP30_dem.tif").is_file());
    let inspection = reopened
        .standalone_dem(&StandaloneDemRequest {
            request_id: "inspect-existing-output".into(),
            dataset: "COP30".into(),
            provider: "COPERNICUS_AWS".into(),
            operation: DemTaskOperation::Inspect,
            input_path: root.join("output/COP30_dem.tif"),
            output_root: root.join("output"),
            bbox: None,
            source_vertical_datum: Some("EGM2008".into()),
            target_vertical_datum: Some("WGS84_ELLIPSOID".into()),
            output_mode: Some("sarscape".into()),
        })
        .unwrap();
    assert_eq!(inspection["result"]["readable"], true);
    assert_eq!(inspection["result"]["metadata"]["width"], 36);
    assert_eq!(inspection["result"]["metadata"]["height"], 36);
    fs::write(
        root.join("download-only-inspection.json"),
        serde_json::to_vec_pretty(&inspection).unwrap(),
    )
    .unwrap();
    reopened.shutdown().unwrap();
}

#[test]
#[ignore = "requires product runtime, existing COP30 input and fresh evidence root"]
fn local_ellipsoid_conversion_then_sarscape_export_native_outputs() {
    let root = PathBuf::from(std::env::var_os("INSAR_COMPLETION_LOCAL_ROOT").expect("fresh root"));
    let input =
        PathBuf::from(std::env::var_os("INSAR_COMPLETION_LOCAL_INPUT").expect("COP30 raw input"));
    let runtime =
        PathBuf::from(std::env::var_os("INSAR_COMPLETION_GIS_ROOT").expect("product runtime"));
    assert!(root.is_absolute() && !root.exists());
    assert!(input.is_absolute() && input.is_file());
    fs::create_dir_all(&root).unwrap();
    let service = state(root.join("tasks.sqlite3"), runtime).unwrap();
    let ellipsoid_root = root.join("ellipsoid-only");
    let export_root = root.join("sarscape-only");
    let request =
        |id: &str, input_path: PathBuf, output_root: PathBuf, source: &str, mode: &str| {
            StandaloneDemRequest {
                request_id: id.into(),
                dataset: "COP30".into(),
                provider: "COPERNICUS_AWS".into(),
                operation: DemTaskOperation::Convert,
                input_path,
                output_root,
                bbox: None,
                source_vertical_datum: Some(source.into()),
                target_vertical_datum: Some("WGS84_ELLIPSOID".into()),
                output_mode: Some(mode.into()),
            }
        };
    let conversion = service
        .standalone_dem(&request(
            "local-ellipsoid",
            input,
            ellipsoid_root.clone(),
            "EGM2008",
            "ellipsoid",
        ))
        .unwrap();
    assert_eq!(conversion["ok"], true);
    let ellipsoid = ellipsoid_root.join("COP30_ellipsoid.tif");
    assert!(ellipsoid.is_file());
    assert!(
        !ellipsoid_root.join("COP30_dem").exists(),
        "ellipsoid-only path must not export SARscape"
    );
    let export = service
        .standalone_dem(&request(
            "local-sarscape",
            ellipsoid.clone(),
            export_root.clone(),
            "WGS84_ELLIPSOID",
            "sarscape",
        ))
        .unwrap();
    assert_eq!(export["ok"], true);
    let mut inspections = Vec::new();
    for path in [
        ellipsoid,
        export_root.join("COP30_ellipsoid.tif"),
        export_root.join("COP30_dem"),
    ] {
        let mut inspect = request(
            "inspect-local",
            path,
            root.clone(),
            "WGS84_ELLIPSOID",
            "sarscape",
        );
        inspect.operation = DemTaskOperation::Inspect;
        let result = service.standalone_dem(&inspect).unwrap();
        assert_eq!(result["result"]["readable"], true);
        assert_eq!(result["result"]["metadata"]["width"], 36);
        assert_eq!(result["result"]["metadata"]["height"], 36);
        inspections.push(result);
    }
    for suffix in ["hdr", "sml"] {
        assert!(
            fs::metadata(export_root.join(format!("COP30_dem.{suffix}")))
                .unwrap()
                .len()
                > 0
        );
    }
    fs::write(
        root.join("local-results.json"),
        serde_json::to_vec_pretty(&json!({
            "conversion": conversion, "export": export, "inspections": inspections,
            "boundary": "production native service; not Workbench/Tauri UI acceptance"
        }))
        .unwrap(),
    )
    .unwrap();
    assert!(
        service.core_restore(true).unwrap().core.tasks.is_empty(),
        "local operations must not create SAR/download tasks"
    );
    service.shutdown().unwrap();
}
