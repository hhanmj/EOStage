use std::fs;
use std::path::PathBuf;

use insar_core::{
    DownloadEngineSnapshot, DownloadItemProgress, DownloadItemResult, DownloadItemStatus,
};
use serde_json::Value;

fn fixture() -> Value {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join("tests/fixtures/v3_download_engine_contracts.json");
    serde_json::from_str(&fs::read_to_string(path).unwrap()).unwrap()
}

#[test]
fn download_progress_and_results_round_trip_the_shared_fixture() {
    let fixture = fixture();
    let progress: DownloadItemProgress =
        serde_json::from_value(fixture["progress"].clone()).unwrap();
    let completed: DownloadItemResult =
        serde_json::from_value(fixture["completed"].clone()).unwrap();
    let failed: DownloadItemResult = serde_json::from_value(fixture["failed"].clone()).unwrap();

    progress.validate().unwrap();
    completed.validate().unwrap();
    failed.validate().unwrap();
    assert_eq!(progress.status, DownloadItemStatus::Active);
    assert!(completed.failure.is_none());
    assert_eq!(failed.failure.as_ref().unwrap().code, "DL005");
    assert_eq!(serde_json::to_value(progress).unwrap(), fixture["progress"]);
    assert_eq!(
        serde_json::to_value(completed).unwrap(),
        fixture["completed"]
    );
    assert_eq!(serde_json::to_value(failed).unwrap(), fixture["failed"]);
}

#[test]
fn engine_snapshot_rejects_non_finite_speed() {
    let mut progress: DownloadItemProgress =
        serde_json::from_value(fixture()["progress"].clone()).unwrap();
    progress.bytes_per_second = f64::NAN;
    assert_eq!(
        progress.validate().unwrap_err(),
        "bytes_per_second must be finite"
    );

    let snapshot = DownloadEngineSnapshot {
        task_id: "task-s1-001".to_string(),
        revision: 1,
        progress: vec![progress],
        results: Vec::new(),
    };
    assert!(snapshot.validate().is_err());
}
