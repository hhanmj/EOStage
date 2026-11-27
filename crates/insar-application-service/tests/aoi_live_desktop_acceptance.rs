use std::time::{Duration, Instant};

use insar_application_service::{
    AdminBoundarySearchRequest, ApplicationService, ApplicationServiceConfig,
};

#[test]
#[ignore = "explicit live DataV/ApplicationService diagnostic"]
fn hubei_boundary_crosses_the_real_application_service_without_hanging() {
    let service = ApplicationService::new(ApplicationServiceConfig::liveness_only());
    let started = Instant::now();
    let result = service
        .admin_boundary_search(&AdminBoundarySearchRequest {
            province: "湖北省".to_owned(),
            city: "全部".to_owned(),
            district: "全部".to_owned(),
            provider_object_id: Some("420000".to_owned()),
            ..Default::default()
        })
        .expect("湖北省 boundary crosses ApplicationService");

    assert_eq!(result.provider, "datav");
    assert_eq!(result.results.len(), 1);
    assert_eq!(result.results[0].provider_object_id, "420000");
    assert!(result.results[0].geometry.is_some());
    assert!(
        started.elapsed() < Duration::from_secs(15),
        "live 湖北省 boundary exceeded the provider timeout envelope"
    );
}

#[test]
#[ignore = "explicit live DataV/ApplicationService/Store diagnostic"]
fn hubei_boundary_crosses_the_real_persistent_store_without_hanging() {
    let store_path = std::env::temp_dir().join(format!(
        "insar-aoi-live-hubei-{}.sqlite3",
        std::process::id()
    ));
    let _ = std::fs::remove_file(&store_path);
    let service = ApplicationService::new(ApplicationServiceConfig::business_store(&store_path));
    let started = Instant::now();
    let result = service
        .admin_boundary_search(&AdminBoundarySearchRequest {
            province: "湖北省".to_owned(),
            city: "全部".to_owned(),
            district: "全部".to_owned(),
            provider_object_id: Some("420000".to_owned()),
            ..Default::default()
        })
        .expect("湖北省 boundary crosses the persistent Store");

    assert_eq!(result.provider, "datav");
    assert_eq!(result.results[0].provider_object_id, "420000");
    assert_eq!(result.results[0].cache_status, "miss");
    assert!(
        started.elapsed() < Duration::from_secs(15),
        "live 湖北省 Store path exceeded the provider timeout envelope"
    );
    drop(service);
    let _ = std::fs::remove_file(&store_path);
}

#[test]
#[ignore = "explicit live Taiwan Nominatim/ApplicationService/Store diagnostic"]
fn taiwan_nantou_crosses_the_real_persistent_store_without_candidates() {
    let store_path = std::env::temp_dir().join(format!(
        "insar-aoi-live-taiwan-nantou-{}.sqlite3",
        std::process::id()
    ));
    let _ = std::fs::remove_file(&store_path);
    let service = ApplicationService::new(ApplicationServiceConfig::business_store(&store_path));
    let result = service
        .admin_boundary_search(&AdminBoundarySearchRequest {
            province: "台湾省".to_owned(),
            city: "南投县".to_owned(),
            district: "全部".to_owned(),
            provider_object_id: None,
            limit: Some(8),
            query: String::new(),
        })
        .expect("南投县 crosses the Taiwan-specific persistent Store route");

    assert_eq!(result.provider, "nominatim");
    assert_eq!(result.warning, None);
    assert_eq!(result.results.len(), 1);
    assert_eq!(result.results[0].country_code.as_deref(), Some("TW"));
    assert!(result.results[0].geometry.is_some());
    drop(service);
    let _ = std::fs::remove_file(&store_path);
}
