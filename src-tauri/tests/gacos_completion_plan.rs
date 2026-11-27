//! Current native ApplicationService planning, no provider submission or fixture Worker.
use eostage_desktop::dem_live_acceptance_support::state;
use serde_json::json;
use std::{fs, path::PathBuf};

#[test]
#[ignore = "requires explicit product runtime and fresh isolated evidence root"]
fn frozen_manual_gacos_plan_uses_native_service_without_submission() {
    let root = PathBuf::from(
        std::env::var_os("INSAR_GACOS_PLAN_EVIDENCE_ROOT").expect("fresh evidence root"),
    );
    let runtime =
        PathBuf::from(std::env::var_os("INSAR_COMPLETION_GIS_ROOT").expect("product runtime"));
    assert!(root.is_absolute() && !root.exists());
    fs::create_dir_all(root.join("output")).unwrap();
    let service = state(root.join("tasks.sqlite3"), runtime).unwrap();
    let before = service.core_restore(true).unwrap();
    assert!(before.core.tasks.is_empty());
    for format in ["geotiff", "binary"] {
        // Same JSON shape sent by Workbench onGacosPlan/planGacosRequest after
        // undefined properties are omitted by IPC serialization. No current AOI.
        let request = json!({
            "request_id": format!("native-gacos-plan-{format}"),
            "source_kind": "manual_dates", "manual_dates": ["20260801", "20260813"],
            "hour": 10, "minute": 29, "aoi_source": "frozen_import",
            "aoi_geojson": {"type":"Polygon","coordinates":[[[100.0,30.0],[100.01,30.0],[100.01,30.01],[100.0,30.01],[100.0,30.0]]]},
            "aoi_name": "冻结试验范围", "region_safe_name": "冻结试验范围", "region_id": null,
            "output_format": format, "output_dir": root.join("output"), "scenes": []
        });
        let result = service
            .gacos_plan(&request)
            .expect("native offline plan must succeed");
        assert_eq!(result["ok"], true);
        assert_eq!(result["offline"], true);
        assert_eq!(result["report"]["has_errors"], false);
        let plan = &result["plan"];
        assert_eq!(plan["requires_credentials"], false);
        assert_eq!(plan["source_kind"], "manual_dates");
        assert_eq!(plan["unique_dates"], json!(["2026-08-01", "2026-08-13"]));
        assert_eq!(plan["output_format"], format);
        let batches = plan["batches"].as_array().unwrap();
        assert_eq!(batches.len(), 1);
        let requests = batches[0]["requests"].as_array().unwrap();
        assert_eq!(requests.len(), 1);
        assert_eq!(requests[0]["form_fields"]["date"], "20260801\n20260813");
        assert_eq!(requests[0]["form_fields"]["H"], "10");
        assert_eq!(requests[0]["form_fields"]["M"], "29");
        fs::write(
            root.join(format!("{format}-plan.json")),
            serde_json::to_vec_pretty(&result).unwrap(),
        )
        .unwrap();
    }
    assert!(
        service.core_restore(true).unwrap().core.tasks.is_empty(),
        "planning must not create or submit a task"
    );
    service.shutdown().unwrap();
}
