use insar_core::{
    ResourceDescriptor, ResourceRegistry, SearchFilter, SearchFilterSchemaCatalog,
    EMBEDDED_SEARCH_FILTER_SCHEMAS_JSON,
};

fn catalog() -> SearchFilterSchemaCatalog {
    SearchFilterSchemaCatalog::embedded().expect("embedded catalog is valid")
}

#[test]
fn embedded_catalog_is_valid_and_owned_by_registry_resources() {
    let registry = ResourceRegistry::embedded().unwrap();
    let catalog = catalog();
    catalog.validate_against_registry(&registry).unwrap();

    // Pilot scope: only optical catalogue products are schema-driven so far
    // (B5 added HLS L30/S30). SAR search schemas are still owned by the UI
    // capability projection.
    let mut resources: Vec<&str> = catalog
        .schemas
        .iter()
        .map(|schema| schema.resource_id.as_str())
        .collect();
    resources.sort_unstable();
    assert_eq!(
        resources,
        [
            "science.hls.l30.v2",
            "science.hls.s30.v2",
            "science.landsat.c2-l2.v1",
            "science.sentinel-2.l1c.v1",
            "science.sentinel-2.l2a.v1",
            "science.sentinel-3.standard.v1",
        ]
    );
    for schema in &catalog.schemas {
        let Some(ResourceDescriptor::ScienceDataset { science, .. }) =
            registry.resource(&schema.resource_id)
        else {
            panic!("schema resource must be a science dataset");
        };
        assert_eq!(
            science.search_schema_id.as_deref(),
            Some(schema.schema_id.as_str())
        );
    }
}

#[test]
fn pilot_schemas_keep_the_current_catalogue_filter_vocabulary() {
    let catalog = catalog();
    let sentinel2 = catalog.for_resource("science.sentinel-2.l2a.v1").unwrap();
    assert_eq!(sentinel2.schema_id, "search.optical.sentinel-2.l2a.v1");
    assert_eq!(
        sentinel2.query_keys(),
        ["spacecraft", "product_type", "datetime", "cloud_cover_lte"]
    );
    let landsat = catalog.schema("search.optical.landsat.c2-l2.v1").unwrap();
    assert_eq!(landsat.resource_id, "science.landsat.c2-l2.v1");
    assert_eq!(
        landsat.query_keys(),
        [
            "spacecraft",
            "processing_levels",
            "datetime",
            "cloud_cover_lte"
        ]
    );
    let platform_values = |schema: &insar_core::SearchFilterSchema| -> Vec<String> {
        schema
            .filters
            .iter()
            .find(|filter| filter.filter_id() == "platform")
            .unwrap()
            .options()
            .iter()
            .map(|option| option.value.clone())
            .collect()
    };
    assert_eq!(
        platform_values(sentinel2),
        ["sentinel_2a", "sentinel_2b", "sentinel_2c"]
    );
    // Milestone 3: Landsat 4/5/7/8/9 are reviewed and admitted for search.
    assert_eq!(
        platform_values(landsat),
        [
            "landsat_4",
            "landsat_5",
            "landsat_7",
            "landsat_8",
            "landsat_9"
        ]
    );
}

#[test]
fn contract_carries_no_provider_identity_or_secrets() {
    let text = EMBEDDED_SEARCH_FILTER_SCHEMAS_JSON.to_ascii_lowercase();
    for forbidden in [
        "provider_id",
        "native_collection_id",
        "endpoint",
        "http",
        "credential",
        "token",
        "password",
        "copernicus.dataspace",
        "planetary-computer",
        "earth-search",
        "usgs.eros",
        "eo:cloud_cover",
        "landsat_ot_c2_l2",
    ] {
        assert!(
            !text.contains(forbidden),
            "contract must not contain {forbidden}"
        );
    }
}

fn first_filter_mut<'a>(
    catalog: &'a mut SearchFilterSchemaCatalog,
    filter_id: &str,
) -> &'a mut SearchFilter {
    catalog.schemas[0]
        .filters
        .iter_mut()
        .find(|filter| filter.filter_id() == filter_id)
        .unwrap()
}

#[test]
fn structural_validation_rejects_ambiguous_or_out_of_range_schemas() {
    let base = catalog();

    let mut duplicate_filter = base.clone();
    let copy = duplicate_filter.schemas[0].filters[0].clone();
    duplicate_filter.schemas[0].filters.push(copy);
    assert!(duplicate_filter.validate().is_err());

    let mut duplicate_key = base.clone();
    if let SearchFilter::Fixed { query_key, .. } =
        first_filter_mut(&mut duplicate_key, "instrument")
    {
        *query_key = Some("spacecraft".to_owned());
    } else {
        panic!("instrument is a fixed fact");
    }
    assert!(duplicate_key.validate().unwrap_err().contains("twice"));

    let mut bad_default = base.clone();
    if let SearchFilter::MultiEnum { default, .. } = first_filter_mut(&mut bad_default, "platform")
    {
        *default = vec!["landsat_8".to_owned()];
    }
    assert!(bad_default.validate().is_err());

    let mut bad_range = base.clone();
    if let SearchFilter::Number {
        minimum, maximum, ..
    } = first_filter_mut(&mut bad_range, "cloud_cover")
    {
        *minimum = 50.0;
        *maximum = 10.0;
    }
    assert!(bad_range.validate().is_err());

    let mut negative = base.clone();
    if let SearchFilter::Number { minimum, .. } = first_filter_mut(&mut negative, "cloud_cover") {
        *minimum = -1.0;
    }
    assert!(negative.validate().is_err());

    let mut two_fixed = base.clone();
    if let SearchFilter::Fixed { options, .. } = first_filter_mut(&mut two_fixed, "instrument") {
        let extra = options[0].clone();
        options.push(insar_core::SearchFilterOption {
            value: "other".to_owned(),
            ..extra
        });
    }
    assert!(two_fixed.validate().is_err());

    let mut bad_key = base.clone();
    if let SearchFilter::DateRange { query_key, .. } =
        first_filter_mut(&mut bad_key, "acquisition_time")
    {
        *query_key = "Item/Datetime".to_owned();
    }
    assert!(bad_key.validate().is_err());

    let mut duplicate_resource = base.clone();
    let second = duplicate_resource.schemas[0].clone();
    duplicate_resource
        .schemas
        .push(insar_core::SearchFilterSchema {
            schema_id: "search.optical.copy.v1".to_owned(),
            ..second
        });
    assert!(duplicate_resource.validate().is_err());
}

#[test]
fn parsing_rejects_unknown_fields_and_controls() {
    let with_provider = EMBEDDED_SEARCH_FILTER_SCHEMAS_JSON.replacen(
        "\"dataset_label\"",
        "\"provider_id\": \"copernicus\", \"dataset_label\"",
        1,
    );
    assert!(SearchFilterSchemaCatalog::from_json(&with_provider).is_err());

    let with_script = EMBEDDED_SEARCH_FILTER_SCHEMAS_JSON.replacen(
        "\"control\": \"date_range\"",
        "\"control\": \"expression\"",
        1,
    );
    assert!(SearchFilterSchemaCatalog::from_json(&with_script).is_err());
}

#[test]
fn registry_cross_check_rejects_unowned_schemas_and_unknown_platforms() {
    let registry = ResourceRegistry::embedded().unwrap();

    let mut unknown_resource = catalog();
    unknown_resource.schemas[0].resource_id = "science.not-registered.v1".to_owned();
    assert!(unknown_resource
        .validate_against_registry(&registry)
        .is_err());

    let mut non_science = catalog();
    non_science.schemas[0].resource_id = "dem.cop30.v1".to_owned();
    assert!(non_science.validate_against_registry(&registry).is_err());

    let mut wrong_schema = catalog();
    wrong_schema.schemas[0].schema_id = "search.optical.sentinel-2.l2a.v2".to_owned();
    assert!(wrong_schema
        .validate_against_registry(&registry)
        .unwrap_err()
        .contains("does not declare"));

    let mut unknown_platform = catalog();
    if let SearchFilter::MultiEnum { options, .. } =
        first_filter_mut(&mut unknown_platform, "platform")
    {
        options[0].registry_platform = Some("Sentinel-2Z".to_owned());
    }
    assert!(unknown_platform
        .validate_against_registry(&registry)
        .unwrap_err()
        .contains("does not declare"));
}

#[test]
fn workbench_placement_is_unique_and_matches_the_registry() {
    let registry = ResourceRegistry::embedded().unwrap();
    let catalog = catalog();
    let optical: Vec<(&str, u32)> = catalog
        .schemas
        .iter()
        .filter_map(|schema| schema.workbench.as_ref())
        .map(|workbench| (workbench.dataset_key.as_str(), workbench.order))
        .collect();
    assert_eq!(
        optical,
        [
            ("sentinel2_l2a", 10),
            ("sentinel2_l1c", 15),
            ("landsat_c2_l2", 20),
            ("sentinel3", 25),
            ("hls_l30", 30),
            ("hls_s30", 40)
        ]
    );

    let mut duplicate_key = catalog.clone();
    duplicate_key.schemas[1]
        .workbench
        .as_mut()
        .unwrap()
        .dataset_key = "sentinel2_l2a".to_owned();
    assert!(duplicate_key
        .validate()
        .unwrap_err()
        .contains("duplicate workbench dataset_key"));

    let mut duplicate_order = catalog.clone();
    duplicate_order.schemas[1].workbench.as_mut().unwrap().order = 10;
    assert!(duplicate_order
        .validate()
        .unwrap_err()
        .contains("reuses workbench order"));

    let mut bad_key = catalog.clone();
    bad_key.schemas[0].workbench.as_mut().unwrap().dataset_key = "Sentinel-2".to_owned();
    assert!(bad_key
        .validate()
        .unwrap_err()
        .contains("invalid workbench dataset_key"));

    let mut wrong_dataset = catalog.clone();
    wrong_dataset.schemas[0]
        .workbench
        .as_mut()
        .unwrap()
        .logical_dataset = "optical.landsat.c2.l2".to_owned();
    assert!(wrong_dataset
        .validate_against_registry(&registry)
        .unwrap_err()
        .contains("differs from the Registry"));
}

#[test]
fn query_values_are_checked_against_the_schema() {
    let catalog = catalog();
    let hls = catalog.schema("search.optical.hls.l30.v2").unwrap();
    let base = serde_json::json!({
        "spacecraft": ["landsat_8", "landsat_9"],
        "cloud_cover_lte": 30,
        "datetime": {"start": "2024-06-01T00:00:00Z", "end": "2024-06-30T23:59:59Z"}
    });
    let filters = |patch: serde_json::Value| {
        let mut value = base.clone();
        for (key, item) in patch.as_object().unwrap() {
            if item.is_null() {
                value.as_object_mut().unwrap().remove(key);
            } else {
                value[key] = item.clone();
            }
        }
        value.as_object().unwrap().clone()
    };
    hls.validate_query_filters(&filters(serde_json::json!({})))
        .unwrap();
    // Omitted optional filters are fine.
    hls.validate_query_filters(&filters(
        serde_json::json!({"spacecraft": null, "cloud_cover_lte": null, "datetime": null}),
    ))
    .unwrap();
    for (patch, needle) in [
        (serde_json::json!({"spacecraft": ["landsat_7"]}), "outside"),
        (
            serde_json::json!({"spacecraft": ["landsat_8", "landsat_8"]}),
            "outside",
        ),
        (serde_json::json!({"spacecraft": []}), "non-empty"),
        (serde_json::json!({"spacecraft": "landsat_8"}), "non-empty"),
        (
            serde_json::json!({"cloud_cover_lte": 500}),
            "number from 0 to 100",
        ),
        (
            serde_json::json!({"cloud_cover_lte": "30"}),
            "number from 0 to 100",
        ),
        (
            serde_json::json!({"datetime": {"start": "2024-06-01T00:00:00Z"}}),
            "start and end",
        ),
        (
            serde_json::json!({"datetime": {"start": "2024-06-30T00:00:00Z", "end": "2024-06-01T00:00:00Z"}}),
            "start before end",
        ),
        (
            serde_json::json!({"datetime": {"start": "2024/06/01", "end": "2024-06-30T00:00:00Z"}}),
            "start before end",
        ),
        (
            serde_json::json!({"processing_levels": ["l2sp"]}),
            "not declared",
        ),
        (
            serde_json::json!({"datetime": {"start": "2024-06-01T08:00:00+08:00", "end": "2024-06-30T00:00:00Z"}}),
            "start before end",
        ),
    ] {
        let error = hls
            .validate_query_filters(&filters(patch.clone()))
            .unwrap_err();
        assert!(error.contains(needle), "{patch}: {error}");
    }

    // A restore_all multi-select is never sent empty.
    let landsat = catalog.schema("search.optical.landsat.c2-l2.v1").unwrap();
    let mut landsat_filters = serde_json::Map::new();
    for filter in &landsat.filters {
        if let insar_core::SearchFilter::Fixed {
            query_key: Some(key),
            options,
            ..
        } = filter
        {
            landsat_filters.insert(key.clone(), serde_json::json!(options[0].value));
        }
    }
    // Restore-all selections are required.
    assert!(landsat
        .validate_query_filters(&landsat_filters)
        .unwrap_err()
        .contains("is required"));
    for filter in &landsat.filters {
        if let insar_core::SearchFilter::MultiEnum {
            query_key,
            options,
            empty_selection: insar_core::EmptySelection::RestoreAll,
            ..
        } = filter
        {
            let all: Vec<&str> = options.iter().map(|option| option.value.as_str()).collect();
            landsat_filters.insert(query_key.clone(), serde_json::json!(all));
        }
    }
    landsat.validate_query_filters(&landsat_filters).unwrap();
    landsat_filters.insert("processing_levels".to_owned(), serde_json::json!([]));
    assert!(landsat
        .validate_query_filters(&landsat_filters)
        .unwrap_err()
        .contains("non-empty"));
}

#[test]
fn every_control_sits_in_a_group_the_workbench_can_render() {
    let catalog = catalog();
    let mut wrong = catalog.clone();
    for filter in &mut wrong.schemas[0].filters {
        if let insar_core::SearchFilter::DateRange { group, .. } = filter {
            *group = insar_core::SearchFilterGroup::Facet;
        }
    }
    assert!(wrong
        .validate()
        .unwrap_err()
        .contains("belong to the facet group"));
    let mut wrong = catalog.clone();
    for filter in &mut wrong.schemas[0].filters {
        if let insar_core::SearchFilter::MultiEnum { group, .. } = filter {
            *group = insar_core::SearchFilterGroup::Quality;
        }
    }
    assert!(wrong
        .validate()
        .unwrap_err()
        .contains("belong to the facet group"));
}
