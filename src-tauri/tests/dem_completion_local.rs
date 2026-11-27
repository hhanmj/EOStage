//! Explicit native ApplicationService validation, not a Workbench click test.
use eostage_desktop::dem_live_acceptance_support::state;
use insar_application_service::{DemTaskOperation, StandaloneDemRequest};
use std::{fs, path::PathBuf};

#[test]
#[ignore = "requires an explicit product GIS runtime, fixture and fresh evidence root"]
fn local_ellipsoid_and_sarscape_use_production_native_port() {
    let runtime = PathBuf::from(std::env::var_os("INSAR_COMPLETION_GIS_ROOT").expect("runtime"));
    let input = PathBuf::from(std::env::var_os("INSAR_COMPLETION_DEM_INPUT").expect("fixture"));
    let root = PathBuf::from(
        std::env::var_os("INSAR_COMPLETION_LOCAL_ROOT").expect("fresh evidence root"),
    );
    assert!(root.is_absolute() && !root.exists());
    fs::create_dir_all(&root).unwrap();
    let service = state(root.join("tasks.sqlite3"), runtime).unwrap();
    let mut reports = Vec::new();
    for mode in ["ellipsoid", "sarscape"] {
        let output = root.join(mode);
        fs::create_dir(&output).unwrap();
        let result = service
            .standalone_dem(&StandaloneDemRequest {
                request_id: format!("completion-local-{mode}"),
                dataset: "USER_LOCAL".into(),
                provider: "LOCAL_FILE".into(),
                operation: DemTaskOperation::Convert,
                input_path: input.clone(),
                output_root: output.clone(),
                bbox: None,
                source_vertical_datum: Some("EGM96".into()),
                target_vertical_datum: Some("WGS84_ELLIPSOID".into()),
                output_mode: Some(mode.into()),
            })
            .expect("production native conversion");
        assert_eq!(result["ok"], true);
        assert_eq!(result["native"], true);
        let files: Vec<_> = fs::read_dir(&output)
            .unwrap()
            .map(|entry| entry.unwrap().path())
            .collect();
        let rasters: Vec<_> = files
            .iter()
            .filter(|path| {
                path.extension().is_some_and(|ext| ext == "tif")
                    || path
                        .file_name()
                        .unwrap()
                        .to_string_lossy()
                        .ends_with("_dem")
            })
            .collect();
        assert_eq!(rasters.len(), if mode == "ellipsoid" { 1 } else { 2 });
        for raster in rasters {
            let inspection = service
                .standalone_dem(&StandaloneDemRequest {
                    request_id: format!("inspect-{mode}"),
                    dataset: "USER_LOCAL".into(),
                    provider: "LOCAL_FILE".into(),
                    operation: DemTaskOperation::Inspect,
                    input_path: raster.clone(),
                    output_root: output.clone(),
                    bbox: None,
                    source_vertical_datum: Some("WGS84_ELLIPSOID".into()),
                    target_vertical_datum: Some("WGS84_ELLIPSOID".into()),
                    output_mode: Some(mode.into()),
                })
                .unwrap();
            assert_eq!(inspection["result"]["readable"], true);
            reports.push(inspection);
        }
        reports.push(result);
    }
    service.shutdown().unwrap();
    fs::write(
        root.join("native-local-results.json"),
        serde_json::to_vec_pretty(&reports).unwrap(),
    )
    .unwrap();
}
