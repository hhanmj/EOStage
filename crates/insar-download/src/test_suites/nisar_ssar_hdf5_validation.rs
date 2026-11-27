use std::fs;
use std::sync::atomic::{AtomicBool, Ordering};

use super::nisar_ssar_hdf5_support::{
    create_valid, create_valid_with_geolocation_shape, item_for_coverage, item_for_pole, mutate,
    FixtureDirectory, Mutation, BASE_ITEM,
};
use crate::nisar_ssar_hdf5_validation::{NisarSsarHdf5Validator, NisarSsarRslcValidationContext};
use crate::{
    establish_hdf5_plugin_isolation, ArtifactValidationError, ArtifactValidationErrorKind,
    ArtifactValidationIssue,
};

fn provider_polarization(item: &str) -> &'static str {
    match item.split('_').nth(9).expect("fixture polarization token") {
        "SHNA" => "SH",
        "SVNA" => "SV",
        "DHNA" => "DH",
        "DVNA" => "DV",
        "CLNA" | "CRNA" => "CP",
        other => panic!("unsupported fixture polarization token: {other}"),
    }
}

fn context(path: &std::path::Path, item: &str) -> NisarSsarRslcValidationContext {
    NisarSsarRslcValidationContext::from_frozen_bhoonidhi(
        item,
        format!("{item}.h5"),
        fs::metadata(path).expect("fixture metadata").len(),
        "1.4.00",
        "W",
        provider_polarization(item),
        true,
    )
    .expect("frozen SSAR context")
}

fn validate(path: &std::path::Path, item: &str) -> Result<(), ArtifactValidationError> {
    establish_hdf5_plugin_isolation().expect("HDF5 plugin isolation");
    NisarSsarHdf5Validator::new().validate_path_with_cancel(path, &context(path, item), &|| false)
}

#[test]
fn all_reviewed_s_band_filename_codes_validate_with_their_declared_channels() {
    for (pole, expected) in [
        ("SHNA", ["HH"].as_slice()),
        ("SVNA", ["VV"].as_slice()),
        ("DHNA", ["HH", "HV"].as_slice()),
        ("DVNA", ["VV", "VH"].as_slice()),
        ("CLNA", ["LH", "LV"].as_slice()),
        ("CRNA", ["RH", "RV"].as_slice()),
    ] {
        let fixtures = FixtureDirectory::new("filename-pole");
        let item = item_for_pole(pole);
        let path = fixtures.item_path(&item);
        create_valid(&path, &item);
        let context = context(&path, &item);
        assert_eq!(
            context
                .polarizations()
                .iter()
                .map(|polarization| polarization.as_str())
                .collect::<Vec<_>>(),
            expected
        );
        validate(&path, &item).expect("supported S-band fixture validates");
    }
}

#[test]
fn provider_polarization_must_match_the_reviewed_filename_mode() {
    for (pole, expected_provider) in [
        ("SHNA", "SH"),
        ("SVNA", "SV"),
        ("DHNA", "DH"),
        ("DVNA", "DV"),
        ("CLNA", "CP"),
        ("CRNA", "CP"),
    ] {
        let fixtures = FixtureDirectory::new("provider-polarization");
        let item = item_for_pole(pole);
        let path = fixtures.item_path(&item);
        create_valid(&path, &item);
        let size = fs::metadata(&path).expect("fixture metadata").len();
        NisarSsarRslcValidationContext::from_frozen_bhoonidhi(
            &item,
            format!("{item}.h5"),
            size,
            "1.4.00",
            "W",
            expected_provider,
            true,
        )
        .expect("matching provider polarization evidence");

        for mismatched in [
            "SP", "DP", "CP", "SH", "SV", "DH", "DV", "CL", "CR", "UNKNOWN",
        ] {
            if mismatched == expected_provider {
                continue;
            }
            assert!(
                NisarSsarRslcValidationContext::from_frozen_bhoonidhi(
                    &item,
                    format!("{item}.h5"),
                    size,
                    "1.4.00",
                    "W",
                    mismatched,
                    true,
                )
                .is_err(),
                "{pole} must reject mismatched Provider polarization {mismatched}"
            );
        }
    }
}

#[test]
fn filename_coverage_tokens_validate_with_the_matching_provider_flag() {
    for (coverage, provider_partial) in [("P", true), ("F", false)] {
        let fixtures = FixtureDirectory::new("filename-coverage");
        let item = item_for_coverage(coverage);
        let path = fixtures.item_path(&item);
        create_valid(&path, &item);
        let context = NisarSsarRslcValidationContext::from_frozen_bhoonidhi(
            &item,
            format!("{item}.h5"),
            fs::metadata(&path).expect("fixture metadata").len(),
            "1.4.00",
            "W",
            "CP",
            provider_partial,
        )
        .expect("frozen coverage context");
        establish_hdf5_plugin_isolation().expect("HDF5 plugin isolation");
        NisarSsarHdf5Validator::new()
            .validate_path_with_cancel(&path, &context, &|| false)
            .expect("supported coverage fixture validates");
    }
}

#[test]
fn filename_context_rejects_unlisted_poles_and_preserves_opaque_catalog_fields() {
    let fixtures = FixtureDirectory::new("context");
    let item = item_for_pole("CRNA");
    let path = fixtures.item_path(&item);
    create_valid(&path, &item);
    let size = fs::metadata(&path).expect("fixture metadata").len();

    for pole in ["QPNA", "CRCR", "SHSH"] {
        let unsupported = item_for_pole(pole);
        let error = NisarSsarRslcValidationContext::from_frozen_bhoonidhi(
            &unsupported,
            format!("{unsupported}.h5"),
            size,
            "1.4.00",
            "W",
            "CP",
            true,
        )
        .unwrap_err();
        assert_eq!(error.issue(), ArtifactValidationIssue::InvalidContext);
    }

    for (software, mode, polarization) in [
        ("1.5.00", "W", "CP"),
        ("1.4.00", "X", "CP"),
        ("1.4.00", "W", "HH+HV"),
    ] {
        let error = NisarSsarRslcValidationContext::from_frozen_bhoonidhi(
            &item,
            format!("{item}.h5"),
            size,
            software,
            mode,
            polarization,
            true,
        )
        .unwrap_err();
        assert_eq!(error.issue(), ArtifactValidationIssue::InvalidContext);
    }

    let error = NisarSsarRslcValidationContext::from_frozen_bhoonidhi(
        &item,
        format!("{item}.h5"),
        size,
        "1.4.00",
        "W",
        "CP",
        false,
    )
    .unwrap_err();
    assert_eq!(error.issue(), ArtifactValidationIssue::InvalidContext);
}

#[test]
fn identification_frequency_and_channel_contract_fail_closed() {
    let mutations = [
        Mutation::WrongMission,
        Mutation::WrongRadarBand,
        Mutation::WrongProductType,
        Mutation::WrongProductLevel,
        Mutation::WrongSpecification,
        Mutation::WrongRelease,
        Mutation::WrongProductVersion,
        Mutation::WrongGranule,
        Mutation::WrongFrequencies,
        Mutation::FrequencyB,
        Mutation::WrongPolarizations,
        Mutation::MissingDeclaredChannel,
        Mutation::WrongImageType,
        Mutation::WrongImageRank,
        Mutation::LsarRoot,
    ];
    for mutation in mutations {
        let fixtures = FixtureDirectory::new("semantic-contract");
        let item = item_for_pole("CRNA");
        let path = fixtures.item_path(&item);
        create_valid(&path, &item);
        mutate(&path, &item, mutation);
        let error = validate(&path, &item).expect_err("invalid SSAR fixture must fail");
        assert_eq!(error.kind(), ArtifactValidationErrorKind::StructureInvalid);
    }
}

#[test]
fn geolocation_grid_enforces_official_ranks_types_and_dimension_relationships() {
    let mutations = [
        Mutation::WrongEpsgType,
        Mutation::WrongEpsgRank,
        Mutation::WrongCoordinateXType,
        Mutation::WrongCoordinateYShape,
        Mutation::WrongSlantRangeRank,
        Mutation::WrongSlantRangeLength,
        Mutation::WrongZeroDopplerRank,
        Mutation::WrongZeroDopplerLength,
        Mutation::WrongHeightRank,
        Mutation::WrongHeightLength,
    ];
    for mutation in mutations {
        let fixtures = FixtureDirectory::new("geolocation-contract");
        let item = item_for_pole("SHNA");
        let path = fixtures.item_path(&item);
        create_valid(&path, &item);
        mutate(&path, &item, mutation);
        let error = validate(&path, &item).expect_err("invalid geolocation must fail");
        assert_eq!(
            error.issue(),
            ArtifactValidationIssue::GeolocationGridInvalid
        );
    }

    let fixtures = FixtureDirectory::new("geolocation-dimensions");
    let item = item_for_pole("SVNA");
    let path = fixtures.item_path(&item);
    create_valid_with_geolocation_shape(&path, &item, (7, 3, 11));
    validate(&path, &item).expect("arbitrary product dimensions with official relationships");
}

#[test]
fn provider_software_version_is_kept_distinct_from_hdf5_product_version() {
    let fixtures = FixtureDirectory::new("software-version-distinction");
    let item = item_for_pole("CRNA");
    let path = fixtures.item_path(&item);
    create_valid(&path, &item);
    let size = fs::metadata(&path).expect("fixture metadata").len();
    let context = NisarSsarRslcValidationContext::from_frozen_bhoonidhi(
        &item,
        format!("{item}.h5"),
        size,
        "1.4.00",
        "W",
        "CP",
        true,
    )
    .expect("frozen version context");
    assert_eq!(context.catalog_software_version(), "1.4.00");
    assert_eq!(context.product_version(), "1.2.1");
    establish_hdf5_plugin_isolation().expect("HDF5 plugin isolation");
    NisarSsarHdf5Validator::new()
        .validate_path_with_cancel(&path, &context, &|| false)
        .expect("independently specified HDF5 product version validates");
}

#[test]
fn unknown_metadata_is_allowed_after_the_shared_security_walk() {
    let fixtures = FixtureDirectory::new("opaque-fields");
    let item = item_for_pole("DHNA");
    let path = fixtures.item_path(&item);
    create_valid(&path, &item);
    mutate(&path, &item, Mutation::ExtraOpaqueField);
    validate(&path, &item).expect("unspecified fields stay opaque");
}

#[test]
fn shared_hdf5_security_guard_rejects_nonhard_links_and_aliases() {
    for mutation in [
        Mutation::SoftLink,
        Mutation::ExternalLink,
        Mutation::HardLinkAlias,
    ] {
        let fixtures = FixtureDirectory::new("security");
        let item = item_for_pole("CRNA");
        let path = fixtures.item_path(&item);
        create_valid(&path, &item);
        mutate(&path, &item, mutation);
        assert!(validate(&path, &item).is_err());
    }
}

#[test]
fn size_checks_and_cooperative_cancellation_happen_before_semantic_reads() {
    let fixtures = FixtureDirectory::new("size-cancel");
    let item = BASE_ITEM.to_owned();
    let path = fixtures.item_path(&item);
    create_valid(&path, &item);
    let size = fs::metadata(&path).expect("fixture metadata").len();
    let part_path = path.with_file_name(format!("{item}.h5.part"));
    fs::copy(&path, &part_path).expect("copy completed bytes to part path");
    validate(&part_path, &item).expect("full-size part artifact validates before promotion");

    let wrong_size = NisarSsarRslcValidationContext::from_frozen_bhoonidhi(
        &item,
        format!("{item}.h5"),
        size + 1,
        "1.4.00",
        "W",
        "CP",
        true,
    )
    .expect("context with wrong expected size");
    establish_hdf5_plugin_isolation().expect("HDF5 plugin isolation");
    assert_eq!(
        NisarSsarHdf5Validator::new()
            .validate_path_with_cancel(&path, &wrong_size, &|| false)
            .unwrap_err()
            .issue(),
        ArtifactValidationIssue::FinalSizeMismatch
    );

    let cancelled = AtomicBool::new(true);
    let error = NisarSsarHdf5Validator::new()
        .validate_path_with_cancel(&path, &context(&path, &item), &|| {
            cancelled.load(Ordering::SeqCst)
        })
        .unwrap_err();
    assert_eq!(error.kind(), ArtifactValidationErrorKind::Unavailable);
    assert_eq!(error.issue(), ArtifactValidationIssue::ValidationCancelled);

    let deferred = NisarSsarRslcValidationContext::from_frozen_bhoonidhi_deferred_size(
        &item,
        format!("{item}.h5"),
        "1.4.00",
        "W",
        "CP",
        true,
    )
    .expect("deferred-size context");
    assert_eq!(
        NisarSsarHdf5Validator::new()
            .validate_path_with_cancel(&path, &deferred, &|| false)
            .unwrap_err()
            .issue(),
        ArtifactValidationIssue::FinalSizeMismatch
    );
}

#[test]
fn malformed_and_truncated_files_never_pass() {
    let fixtures = FixtureDirectory::new("malformed");
    let item = item_for_pole("SHNA");
    let path = fixtures.item_path(&item);
    fs::write(&path, b"not-an-hdf5-file").expect("write malformed fixture");
    let malformed_context = NisarSsarRslcValidationContext::from_frozen_bhoonidhi(
        &item,
        format!("{item}.h5"),
        16,
        "1.4.00",
        "W",
        "SH",
        true,
    )
    .expect("malformed context");
    establish_hdf5_plugin_isolation().expect("HDF5 plugin isolation");
    assert!(NisarSsarHdf5Validator::new()
        .validate_path_with_cancel(&path, &malformed_context, &|| false)
        .is_err());

    let truncated = fixtures.item_path(&item_for_pole("SVNA"));
    let truncated_item = item_for_pole("SVNA");
    create_valid(&truncated, &truncated_item);
    let original_size = fs::metadata(&truncated).expect("fixture metadata").len();
    let expected = context(&truncated, &truncated_item);
    fs::OpenOptions::new()
        .write(true)
        .open(&truncated)
        .expect("open fixture for truncation")
        .set_len(original_size / 2)
        .expect("truncate fixture");
    assert_eq!(
        NisarSsarHdf5Validator::new()
            .validate_path_with_cancel(&truncated, &expected, &|| false)
            .unwrap_err()
            .issue(),
        ArtifactValidationIssue::FinalSizeMismatch
    );
}

#[test]
fn production_registry_keeps_ssar_binding_separate_and_requires_remote_size() {
    let fixtures = FixtureDirectory::new("registry-binding");
    let path = fixtures.item_path(BASE_ITEM);
    create_valid(&path, BASE_ITEM);
    let size = fs::metadata(&path).unwrap().len();
    let context = NisarSsarRslcValidationContext::from_frozen_bhoonidhi_deferred_size(
        BASE_ITEM,
        format!("{BASE_ITEM}.h5"),
        "1.4.00",
        "W",
        "CP",
        true,
    )
    .unwrap();
    let registry = crate::ArtifactValidatorRegistry::standard();
    let binding = crate::NISAR_SSAR_RSLC_STRUCTURE_VALIDATOR_BINDING;
    let asset = crate::ResolvedAsset::new(
        "https://bhoonidhi-api.nrsc.gov.in/download",
        crate::ResolvedMethod::Get,
    )
    .unwrap()
    .with_expected_size(Some(size));
    establish_hdf5_plugin_isolation().unwrap();
    registry
        .validate_for_test(binding, &path, &asset, context.clone().into())
        .unwrap();
    assert_eq!(
        registry
            .resolve(
                crate::NISAR_RSLC_STRUCTURE_VALIDATOR_V2_BINDING,
                context.clone().into()
            )
            .unwrap_err()
            .issue(),
        ArtifactValidationIssue::InvalidContext,
    );
    let unknown_size = crate::ResolvedAsset::new(
        "https://bhoonidhi-api.nrsc.gov.in/download",
        crate::ResolvedMethod::Get,
    )
    .unwrap();
    assert_eq!(
        registry
            .validate_for_test(binding, &path, &unknown_size, context.into())
            .unwrap_err()
            .issue(),
        ArtifactValidationIssue::InvalidContext,
    );
}
