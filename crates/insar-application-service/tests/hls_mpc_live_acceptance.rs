//! Issue #3 R2 B5: bounded live evidence that a search-only HLS v2.0 product
//! is searched by route-neutral intent through the production application
//! path (planner, route policy, Microsoft STAC adapter, freeze) without any
//! credential, asset URL or transfer authority. Opt-in only.

use std::fs;
use std::path::PathBuf;

use insar_application_service::{
    ApplicationCatalogIntentSearchAndFreezeRequest, ApplicationService, ApplicationServiceConfig,
    CatalogSearchIntent, CdseNetworkMode,
};
use insar_store::StoreWorkbenchPageRequest;
use serde_json::{json, Map, Value};

const LIVE_ROOT_ENV: &str = "INSAR_HLS_MPC_LIVE_ROOT";

struct Workspace {
    root: PathBuf,
    clean_on_drop: bool,
}

impl Drop for Workspace {
    fn drop(&mut self) {
        if self.clean_on_drop {
            let _ = fs::remove_dir_all(&self.root);
        }
    }
}

fn aoi() -> Map<String, Value> {
    // A 0.5 degree box near Yichang, Hubei (MGRS 49RDP/49RCP).
    json!({
        "type": "Polygon",
        "coordinates": [[[110.0, 30.0], [110.5, 30.0], [110.5, 30.5], [110.0, 30.5], [110.0, 30.0]]]
    })
    .as_object()
    .unwrap()
    .clone()
}

fn request(token: &str, platforms: Value) -> ApplicationCatalogIntentSearchAndFreezeRequest {
    ApplicationCatalogIntentSearchAndFreezeRequest {
        request_id: format!("hls-live-{token}"),
        intent: CatalogSearchIntent {
            resource_id: format!("science.hls.{token}.v2"),
            search_schema_id: format!("search.optical.hls.{token}.v2"),
            schema_revision: 1,
            aoi_geojson: Some(aoi()),
            result_limit: Some(10),
            filters: json!({
                "spacecraft": platforms,
                "cloud_cover_lte": 80.0,
                "datetime": {"start": "2024-06-01T00:00:00Z", "end": "2024-06-30T23:59:59Z"}
            })
            .as_object()
            .unwrap()
            .clone(),
        },
        batch_id: format!("hls-live-{token}-batch"),
        created_at: "2026-10-19T00:00:00Z".to_owned(),
        aoi_name: "HLS live AOI".to_owned(),
        aoi_geojson: Some(aoi()),
        workbench_page_size: 10,
    }
}

#[test]
#[ignore = "requires explicit opt-in and public Microsoft Planetary Computer access"]
fn hls_l30_and_s30_live_intent_search_freezes_search_only_scenes() {
    let root = std::env::var_os(LIVE_ROOT_ENV)
        .map(PathBuf::from)
        .expect("HLS_LIVE_ROOT_REQUIRED");
    assert!(
        root.is_absolute() && !root.exists(),
        "HLS_LIVE_ROOT_MUST_BE_ABSENT_ABSOLUTE"
    );
    fs::create_dir_all(&root).unwrap();
    let mut workspace = Workspace {
        root: root.clone(),
        clean_on_drop: false,
    };
    for (token, platforms) in [
        ("l30", json!(["landsat_8", "landsat_9"])),
        ("s30", json!(["sentinel_2a", "sentinel_2b"])),
    ] {
        let service = ApplicationService::new(ApplicationServiceConfig::business_store(
            root.join(format!("{token}.sqlite3")),
        ));
        // Same direct provider network mode as the other Microsoft live tests.
        service
            .set_cdse_network_policy(CdseNetworkMode::Direct)
            .unwrap_or_else(|error| panic!("HLS_LIVE_NETWORK_POLICY_{}", error.code()));
        let frozen = service
            .catalog_intent_search_and_freeze_with_progress(&request(token, platforms), |_| {})
            .unwrap_or_else(|error| panic!("HLS_LIVE_{token}_SEARCH_{}", error.code()));
        let workbench = frozen.core.workbench.expect("HLS live batch");
        let page = service
            .workbench_page(&StoreWorkbenchPageRequest {
                batch_id: format!("hls-live-{token}-batch"),
                expected_revision: workbench.revision,
                page_index: 0,
                page_size: 10,
            })
            .unwrap();
        let mut ids = Vec::new();
        for entry in &page.page.items {
            let item = &entry.item;
            let metadata = &item.metadata;
            assert_eq!(
                metadata.get("logical_dataset"),
                Some(&json!(format!("optical.hls.{token}.v2")))
            );
            assert_eq!(metadata.get("assets"), Some(&json!([])));
            assert!(item.payload.is_empty() && item.execution_policy.is_none());
            let serialized = serde_json::to_string(item).unwrap();
            assert!(!serialized.contains("blob.core.windows.net"));
            assert!(!serialized.contains("token"));
            let properties = metadata.get("properties").cloned().unwrap_or_default();
            let cloud = properties
                .get("cloud_cover")
                .and_then(Value::as_f64)
                .unwrap();
            assert!((0.0..=80.0).contains(&cloud));
            ids.push(format!(
                "{} {} cloud={cloud} {}",
                item.display_name,
                properties
                    .get("platform")
                    .and_then(Value::as_str)
                    .unwrap_or("?"),
                item.acquisition_datetime.as_deref().unwrap_or("?")
            ));
        }
        eprintln!(
            "HLS_LIVE token={token} candidate_count={} page_items={}",
            workbench.candidate_count,
            ids.len()
        );
        for id in &ids {
            eprintln!("HLS_LIVE item {id}");
        }
        assert!(workbench.candidate_count >= 1, "HLS_LIVE_{token}_EMPTY");
        service.shutdown().unwrap();
    }
    workspace.clean_on_drop = true;
    eprintln!("HLS_LIVE outcome=completed credential_input_count=0 workspace_cleaned=true");
}
