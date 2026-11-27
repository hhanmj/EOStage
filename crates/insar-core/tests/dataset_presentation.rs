use insar_core::{project_dataset_presentation, ResourceDescriptor, ResourceRegistry};

#[test]
fn optical_r2_pilot_projects_sentinel2_and_landsat_from_registry() {
    let registry = ResourceRegistry::embedded().unwrap();
    for (resource_id, expected_schema, expected_platforms) in [
        (
            "science.sentinel-2.l2a.v1",
            "search.optical.sentinel-2.l2a.v1",
            vec!["Sentinel-2A", "Sentinel-2B", "Sentinel-2C"],
        ),
        (
            "science.landsat.c2-l2.v1",
            "search.optical.landsat.c2-l2.v1",
            vec![
                "Landsat-4",
                "Landsat-5",
                "Landsat-7",
                "Landsat-8",
                "Landsat-9",
            ],
        ),
    ] {
        let presentation = project_dataset_presentation(&registry, resource_id).unwrap();
        assert_eq!(presentation.resource_id, resource_id);
        assert_eq!(presentation.search_schema_id, expected_schema);
        assert_eq!(presentation.platforms, expected_platforms);
        assert!(!presentation.declared_search_route_ids.is_empty());
        assert!(presentation
            .declared_search_route_ids
            .iter()
            .all(|route| route.starts_with("route.")));
    }
}

#[test]
fn presentation_reflects_registry_facts_instead_of_a_second_frontend_table() {
    let mut registry = ResourceRegistry::embedded().unwrap();
    let resource = registry
        .resources
        .iter_mut()
        .find(|resource| resource.common().resource_id == "science.sentinel-2.l2a.v1")
        .unwrap();
    let ResourceDescriptor::ScienceDataset { science, .. } = resource else {
        panic!("expected science descriptor")
    };
    science.mission = "reviewed-test-mission".to_owned();

    let presentation =
        project_dataset_presentation(&registry, "science.sentinel-2.l2a.v1").unwrap();
    assert_eq!(presentation.mission, "reviewed-test-mission");
}

#[test]
fn projection_does_not_infer_search_or_delivery_capability() {
    let registry = ResourceRegistry::embedded().unwrap();
    assert!(project_dataset_presentation(&registry, "not-a-resource").is_err());
    assert!(project_dataset_presentation(&registry, "dem.cop30.v1").is_err());
    assert!(project_dataset_presentation(&registry, "science.envisat.reserved.v1").is_err());

    let landsat = project_dataset_presentation(&registry, "science.landsat.c2-l2.v1").unwrap();
    // Route declarations are not admission, credentials, transport, or delivery permission.
    let json = serde_json::to_string(&landsat).unwrap();
    for forbidden in [
        "credential",
        "download_ready",
        "delivery_ready",
        "native_collection_id",
    ] {
        assert!(!json.contains(forbidden));
    }
}
