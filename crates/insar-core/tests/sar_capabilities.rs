use insar_core::{asf_catalog_archive_window, asf_catalog_capabilities, FilterValueType};

#[test]
fn asf_catalog_lists_only_the_initial_real_datasets() {
    let response = asf_catalog_capabilities();
    let compatibility_snapshot: insar_core::ProviderCapabilitiesResponse = serde_json::from_str(
        include_str!("../../../contracts/v3/asf-capabilities.v1.json"),
    )
    .expect("ASF compatibility snapshot");
    assert_eq!(response.provider_id, compatibility_snapshot.provider_id);
    assert_eq!(response.available, compatibility_snapshot.available);
    assert_eq!(response.reason, compatibility_snapshot.reason);
    assert_eq!(response.datasets, compatibility_snapshot.datasets);
    let mut source_owned_compatibility = compatibility_snapshot.sources.clone();
    for source in &mut source_owned_compatibility {
        if source.search_kind != insar_core::SarSourceSearchKind::LegacyAsfSearch {
            continue;
        }
        let capability = compatibility_snapshot
            .datasets
            .iter()
            .find(|capability| capability.dataset == source.selector)
            .expect("legacy ASF source capability");
        source.filters.insert(
            "dataset_capabilities".to_owned(),
            serde_json::to_value(capability).expect("serializable DatasetCapabilities"),
        );
    }
    assert_eq!(response.sources, source_owned_compatibility);
    assert!(response
        .sources
        .iter()
        .all(|source| source.provider_id.as_deref() != Some("isro.nrsc.bhoonidhi")));
    assert_eq!(
        response.source_version,
        compatibility_snapshot.source_version
    );
    let datasets = response
        .datasets
        .iter()
        .map(|item| item.dataset.as_str())
        .collect::<Vec<_>>();

    assert!(response.available);
    assert_eq!(response.provider_id, "asf.catalog");
    assert_eq!(response.source_version, "asf-search 12.2.2");
    assert_eq!(
        datasets,
        ["SENTINEL-1", "SLC-BURST", "ALOS PALSAR", "SEASAT",]
    );
    let source_selectors = response
        .sources
        .iter()
        .map(|source| source.selector.as_str())
        .collect::<Vec<_>>();
    assert_eq!(
        source_selectors,
        [
            "SENTINEL-1",
            "SLC-BURST",
            "ALOS PALSAR",
            "SEASAT",
            "ALOS-2",
            "NISAR",
            "OPERA-S1",
        ]
    );
    let opera = response
        .sources
        .iter()
        .find(|source| source.selector == "OPERA-S1")
        .expect("OPERA-S1 typed catalog profile source");
    assert_eq!(
        opera.search_kind,
        insar_core::SarSourceSearchKind::TypedCatalogProfile
    );
    assert_eq!(
        opera.logical_dataset.as_deref(),
        Some("sar.opera.s1.disp.v1")
    );
    assert_eq!(opera.provider_id.as_deref(), Some("asf.nasa.earthdata"));
    assert_eq!(
        opera.native_collection_id.as_deref(),
        Some("C2834279090-ASF")
    );
    assert_eq!(
        opera.collection_profile_id.as_deref(),
        Some("asf.opera.s1.v1")
    );
    assert_eq!(opera.supported_actions, ["search"]);
    assert_eq!(
        opera.filters["options"]["product_type"],
        serde_json::json!(["DISP-S1", "CSLC-S1"])
    );
    assert!(
        !response.datasets.iter().any(|item| item.dataset == "OPERA-S1"),
        "OPERA-S1 stays a typed catalog profile source and never leaks into legacy asf-search dataset capabilities"
    );
    let burst = response
        .datasets
        .iter()
        .find(|item| item.dataset == "SLC-BURST")
        .unwrap();
    assert!(burst.supports_burst);
    assert_eq!(burst.processing_levels, ["BURST"]);
    assert_eq!(burst.product_families, ["BURST"]);
    assert!(burst.frame_filter.is_none());
    assert!(burst.track_filter.is_none());
    assert!(response
        .datasets
        .iter()
        .filter(|item| item.dataset != "SLC-BURST")
        .all(|item| !item.supports_burst));
}

#[test]
fn asf_dataset_capabilities_do_not_mix_mission_vocabularies() {
    let response = asf_catalog_capabilities();
    let sentinel = response
        .datasets
        .iter()
        .find(|item| item.dataset == "SENTINEL-1")
        .unwrap();
    let alos = response
        .datasets
        .iter()
        .find(|item| item.dataset == "ALOS PALSAR")
        .unwrap();
    let seasat = response
        .datasets
        .iter()
        .find(|item| item.dataset == "SEASAT")
        .unwrap();

    assert!(sentinel.processing_levels.contains(&"GRD_HD".to_string()));
    assert!(sentinel.processing_levels.contains(&"GRD_HS".to_string()));
    assert!(!sentinel.processing_levels.contains(&"GRD_FD".to_string()));
    assert!(!sentinel.processing_levels.contains(&"L1.1".to_string()));
    assert!(sentinel.beam_modes.contains(&"IW".to_string()));
    assert!(sentinel.beam_modes.contains(&"SM".to_string()));
    assert!(sentinel.beam_modes.contains(&"S1".to_string()));
    assert!(sentinel.beam_modes.contains(&"S6".to_string()));

    assert!(alos.processing_levels.contains(&"L1.1".to_string()));
    assert!(alos.beam_modes.contains(&"FBS".to_string()));
    assert!(alos.beam_modes.contains(&"DSN".to_string()));
    assert!(alos.polarizations.contains(&"HH 3SCAN".to_string()));
    assert!(alos.polarizations.contains(&"VV+VH".to_string()));
    assert!(alos.polarizations.contains(&"full".to_string()));
    assert!(!alos.processing_levels.contains(&"GRD_HD".to_string()));
    assert!(!alos.beam_modes.contains(&"IW".to_string()));

    // ASF removed the legacy missions (ERS, JERS-1, RADARSAT-1) from its
    // searchable interfaces (Owner evidence 2026-10-31): they are no longer
    // projected as ASF datasets or sources.
    for selector in ["ERS", "JERS-1", "RADARSAT-1"] {
        assert!(
            !response
                .datasets
                .iter()
                .any(|item| item.dataset == selector),
            "{selector} is retired"
        );
        assert!(
            !response
                .sources
                .iter()
                .any(|source| source.selector == selector),
            "{selector} has no source projection"
        );
    }

    assert!(seasat.processing_levels.is_empty());
    assert_eq!(seasat.product_families, ["OTHER"]);
    assert_eq!(seasat.beam_modes, ["STD"]);

    assert_eq!(
        sentinel.frame_filter.as_ref().unwrap().parameter.as_deref(),
        Some("frame")
    );
    assert_eq!(
        seasat.frame_filter.as_ref().unwrap().value_type,
        FilterValueType::PaddedString
    );
    assert_eq!(sentinel.track_filter.as_ref().unwrap().minimum, Some(1));
    assert_eq!(sentinel.track_filter.as_ref().unwrap().maximum, Some(175));
    assert_eq!(seasat.frame_filter.as_ref().unwrap().exact_width, Some(4));
    assert_eq!(seasat.frame_filter.as_ref().unwrap().minimum, Some(208));
    assert_eq!(seasat.frame_filter.as_ref().unwrap().maximum, Some(3458));
    assert!(sentinel.supported_actions.iter().any(|rule| {
        rule.action == "orbit_match"
            && rule
                .required_fields
                .contains(&"acquisition_start".to_string())
    }));
}

#[test]
fn asf_catalog_archive_windows_cover_each_mission_without_sentinel_truncation() {
    let sentinel = asf_catalog_archive_window("SENTINEL-1").expect("Sentinel window");
    assert_eq!(sentinel.start, "2014-10-01T00:00:00Z");
    assert_eq!(sentinel.end, None);

    for (dataset, expected_start, expected_end) in [
        (
            "ALOS PALSAR",
            "2006-01-01T00:00:00Z",
            "2011-12-31T23:59:59Z",
        ),
        ("SEASAT", "1978-01-01T00:00:00Z", "1978-12-31T23:59:59Z"),
    ] {
        let window = asf_catalog_archive_window(dataset).expect(dataset);
        assert_eq!(window.start, expected_start, "{dataset}");
        assert_eq!(window.end.as_deref(), Some(expected_end), "{dataset}");
        assert!(window.start.as_str() < "2014-10-01T00:00:00Z", "{dataset}");
    }

    // ASF removed the legacy missions from its searchable interfaces
    // (Owner evidence 2026-10-31): no archive window is projected for them.
    for dataset in ["ERS", "JERS-1", "RADARSAT-1"] {
        assert!(
            asf_catalog_archive_window(dataset).is_none(),
            "{dataset} is retired and has no archive window"
        );
    }

    assert!(asf_catalog_archive_window("unknown").is_none());
}
