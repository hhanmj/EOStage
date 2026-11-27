//! Empirical Challenger Stress Harness for Milestone 2: 7 Missing Data Sources Route Policies
//! Covers route resolution, collection profile resolution, filter key validation, and adversarial input fuzzing.

use insar_provider::catalog::{
    CollectionProfileRegistry, DatasetRoutePolicyError, DatasetRoutePolicyRegistry, RouteUse,
};

struct RouteFixture {
    name: &'static str,
    logical_dataset: &'static str,
    provider_id: &'static str,
    native_collection_id: &'static str,
    expected_route_id: &'static str,
    expected_profile_id: &'static str,
    supported_filter_keys: &'static [&'static str],
}

const M2_FIXTURES: &[RouteFixture] = &[
    RouteFixture {
        name: "NASA OPERA Displacement Sentinel-1",
        logical_dataset: "sar.opera.s1.disp.v1",
        provider_id: "asf.nasa.earthdata",
        native_collection_id: "C2834279090-ASF",
        expected_route_id: "route.asf.opera-s1.catalog.v1",
        expected_profile_id: "asf.opera.s1.collection.v1",
        supported_filter_keys: &["datetime", "track", "frame", "flight_direction"],
    },
    RouteFixture {
        name: "COMET LiCSAR Global Sentinel-1",
        logical_dataset: "sar.comet.licsar.interferogram.v1",
        provider_id: "comet.licsar.portal",
        native_collection_id: "LiCSAR_products.public",
        expected_route_id: "route.comet.licsar.catalog.v1",
        expected_profile_id: "comet.licsar.collection.v1",
        supported_filter_keys: &["datetime", "track", "frame"],
    },
    RouteFixture {
        name: "ASTER GDEM v3",
        logical_dataset: "dem.aster-gdem.v3",
        provider_id: "asf.nasa.earthdata",
        native_collection_id: "ASTGTM.v003",
        expected_route_id: "route.lpdaac.aster-gdem-v3.earthdata.v1",
        expected_profile_id: "lpdaac.aster-gdem.v3.collection.v1",
        supported_filter_keys: &["datetime"],
    },
    RouteFixture {
        name: "USGS Earthquake Catalog",
        logical_dataset: "event.usgs.earthquake.v1",
        provider_id: "usgs.earthquake.fdsn",
        native_collection_id: "earthquake",
        expected_route_id: "route.usgs.earthquake.fdsn.v1",
        expected_profile_id: "usgs.earthquake.collection.v1",
        supported_filter_keys: &["datetime", "min_magnitude", "max_magnitude", "max_depth_km"],
    },
    RouteFixture {
        name: "NGL GNSS Time Series",
        logical_dataset: "geodesy.ngl.gnss.v1",
        provider_id: "unr.ngl.geodesy",
        native_collection_id: "tenv3",
        expected_route_id: "route.unr.ngl.geodesy.v1",
        expected_profile_id: "unr.ngl.gnss.collection.v1",
        supported_filter_keys: &["station_id", "max_distance_km"],
    },
    RouteFixture {
        name: "ECMWF ERA5 Reanalysis",
        logical_dataset: "atmospheric.era5.reanalysis.v1",
        provider_id: "ecmwf.era5.arco",
        native_collection_id: "ARCO_ERA5_HOURLY",
        expected_route_id: "route.era5.arco.v1",
        expected_profile_id: "ecmwf.era5.arco.collection.v1",
        supported_filter_keys: &["datetime", "pressure_levels"],
    },
    RouteFixture {
        name: "IGS GIM Ionosphere",
        logical_dataset: "ionospheric.igs.gim.v1",
        provider_id: "asf.nasa.earthdata",
        native_collection_id: "CDDIS_IONEX_GIM",
        expected_route_id: "route.cddis.ionex.v1",
        expected_profile_id: "cddis.ionex.gim.collection.v1",
        supported_filter_keys: &["datetime"],
    },
];

#[test]
fn test_all_7_routes_resolve_production_and_fixture() {
    let registry = DatasetRoutePolicyRegistry;
    for fixture in M2_FIXTURES {
        for route_use in [RouteUse::Production, RouteUse::OfflineFixture] {
            let policy = registry
                .resolve(
                    fixture.logical_dataset,
                    fixture.provider_id,
                    fixture.native_collection_id,
                    route_use,
                )
                .unwrap_or_else(|e| {
                    panic!(
                        "Failed to resolve {} with {:?}: {:?}",
                        fixture.name, route_use, e
                    )
                });

            assert_eq!(
                policy.route_id, fixture.expected_route_id,
                "Route ID mismatch for {}",
                fixture.name
            );
            assert_eq!(
                policy.collection_profile_id, fixture.expected_profile_id,
                "Profile ID mismatch for {}",
                fixture.name
            );
            assert!(
                policy.production_enabled,
                "Route must be production enabled for {}",
                fixture.name
            );
            assert_eq!(
                policy.catalogue_aoi_crs, "EPSG:4326",
                "CRS mismatch for {}",
                fixture.name
            );
        }
    }
}

#[test]
fn test_all_7_collection_profiles_resolve_correctly() {
    let registry = CollectionProfileRegistry;
    for fixture in M2_FIXTURES {
        let profile_from_source = registry
            .for_source(
                fixture.logical_dataset,
                fixture.provider_id,
                fixture.native_collection_id,
            )
            .unwrap_or_else(|| {
                panic!(
                    "Failed to find collection profile for source {} ({}, {}, {})",
                    fixture.name,
                    fixture.logical_dataset,
                    fixture.provider_id,
                    fixture.native_collection_id
                )
            });

        assert_eq!(
            profile_from_source.profile_id, fixture.expected_profile_id,
            "Source profile mismatch for {}",
            fixture.name
        );
        assert_eq!(
            profile_from_source.logical_dataset, fixture.logical_dataset,
            "Logical dataset mismatch for {}",
            fixture.name
        );

        let profile_from_dataset = registry
            .for_logical_dataset(fixture.logical_dataset)
            .unwrap_or_else(|| {
                panic!(
                    "Failed to find collection profile for dataset {} ({})",
                    fixture.name, fixture.logical_dataset
                )
            });

        assert_eq!(
            profile_from_dataset.profile_id, fixture.expected_profile_id,
            "Dataset profile mismatch for {}",
            fixture.name
        );
    }
}

#[test]
fn test_all_7_routes_filter_validation_and_rejection() {
    let registry = DatasetRoutePolicyRegistry;
    for fixture in M2_FIXTURES {
        // 1. Supported filters must validate cleanly
        registry
            .validate_filter_keys(fixture.logical_dataset, fixture.supported_filter_keys)
            .unwrap_or_else(|e| {
                panic!(
                    "Supported filter validation failed for {}: {:?}",
                    fixture.name, e
                )
            });

        // 2. Subsets must also validate cleanly
        for key in fixture.supported_filter_keys {
            registry
                .validate_filter_keys(fixture.logical_dataset, &[*key])
                .unwrap_or_else(|e| {
                    panic!(
                        "Single key '{}' validation failed for {}: {:?}",
                        key, fixture.name, e
                    )
                });
        }

        // 3. Unsupported / invented filters must be rejected with UnsupportedFilter
        let adversarial_keys = &[
            "unsupported_key",
            "sql_injection' OR 1=1 --",
            "cloud_cover_lte",
            "__proto__",
            "",
            "   ",
        ];
        for bad_key in adversarial_keys {
            if fixture.supported_filter_keys.contains(bad_key) {
                continue;
            }
            let res = registry.validate_filter_keys(fixture.logical_dataset, &[*bad_key]);
            assert_eq!(
                res.unwrap_err(),
                DatasetRoutePolicyError::UnsupportedFilter((*bad_key).to_owned()),
                "Bad key '{}' was not rejected for {}",
                bad_key,
                fixture.name
            );
        }
    }
}

#[test]
fn test_adversarial_route_fuzzing_and_error_handling() {
    let registry = DatasetRoutePolicyRegistry;

    let long_string = "a".repeat(1000);
    let adversarial_strings = &[
        "",
        "   ",
        "unknown.provider",
        "random_collection_12345",
        "'; DROP TABLE users; --",
        "<script>alert(1)</script>",
        "🌟🛰️🗺️",
        long_string.as_str(),
        "invalid\0nullbyte",
    ];

    for fixture in M2_FIXTURES {
        // Mismatched provider
        for bad_provider in adversarial_strings {
            if *bad_provider == fixture.provider_id {
                continue;
            }
            let res = registry.resolve(
                fixture.logical_dataset,
                bad_provider,
                fixture.native_collection_id,
                RouteUse::Production,
            );
            assert!(
                matches!(
                    res,
                    Err(DatasetRoutePolicyError::ProviderMismatch { .. })
                        | Err(DatasetRoutePolicyError::ProviderUnavailable(_))
                ),
                "Expected ProviderMismatch or ProviderUnavailable for {}, got {:?}",
                fixture.name,
                res
            );
        }

        // Mismatched collection
        for bad_collection in adversarial_strings {
            if *bad_collection == fixture.native_collection_id {
                continue;
            }
            let res = registry.resolve(
                fixture.logical_dataset,
                fixture.provider_id,
                bad_collection,
                RouteUse::Production,
            );
            assert!(
                matches!(
                    res,
                    Err(DatasetRoutePolicyError::NativeCollectionMismatch { .. })
                ),
                "Expected NativeCollectionMismatch for {}, got {:?}",
                fixture.name,
                res
            );
        }
    }

    // Completely unknown logical dataset
    for bad_dataset in adversarial_strings {
        let res = registry.resolve(
            bad_dataset,
            "asf.nasa.earthdata",
            "C2834279090-ASF",
            RouteUse::Production,
        );
        assert!(
            matches!(res, Err(DatasetRoutePolicyError::UnknownLogicalDataset(_))),
            "Expected UnknownLogicalDataset for '{}', got {:?}",
            bad_dataset,
            res
        );
    }
}
