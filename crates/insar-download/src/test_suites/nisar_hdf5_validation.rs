use std::fs;
use std::path::Path;
use std::time::Duration;

use super::nisar_hdf5_support::{
    apply_mutation_to_fixture, create_allowed_filter_fixture, create_huge_logical_fixture,
    create_mutated_fixture, create_valid_fixture, create_valid_fixture_for_profile,
    FixtureDirectory, FixtureMutation, FrequencyScope, LEGACY_PRODUCT_SPECIFICATION_VERSION,
    P05023_PRODUCT_SPECIFICATION_VERSION,
};
use insar_download::{
    ArtifactValidationContext, ArtifactValidationError, ArtifactValidationErrorKind,
    ArtifactValidationIssue, ArtifactValidatorBinding, ArtifactValidatorRegistry,
    NisarRslcValidationContext, ResolvedAsset, ResolvedMethod, NISAR_RSLC_STRUCTURE_VALIDATOR_ID,
    NISAR_RSLC_STRUCTURE_VALIDATOR_V2_BINDING, NISAR_RSLC_STRUCTURE_VALIDATOR_V2_VERSION,
    NISAR_RSLC_STRUCTURE_VALIDATOR_VERSION,
};

use crate::nisar_hdf5_validation::read_1d_call_count_for_test;

fn binding() -> ArtifactValidatorBinding {
    ArtifactValidatorBinding::new_for_test(
        NISAR_RSLC_STRUCTURE_VALIDATOR_ID,
        NISAR_RSLC_STRUCTURE_VALIDATOR_VERSION,
    )
}

fn v2_binding() -> ArtifactValidatorBinding {
    NISAR_RSLC_STRUCTURE_VALIDATOR_V2_BINDING
}

fn validate(path: &Path, scope: FrequencyScope) -> Result<(), ArtifactValidationError> {
    let _test_guard = insar_download::hdf5_plugin_isolation::HDF5_TEST_LOCK
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    insar_download::establish_hdf5_plugin_isolation()
        .expect("app-owned HDF5 plugin isolation is required by the validator");
    validate_with_registry(&ArtifactValidatorRegistry::standard(), path, scope)
}

fn validate_with_registry(
    registry: &ArtifactValidatorRegistry,
    path: &Path,
    scope: FrequencyScope,
) -> Result<(), ArtifactValidationError> {
    let asset = resolved_asset();
    registry.validate_for_test(binding(), path, &asset, validation_context(scope))
}

/// Test-only probe used by the child-process isolation tests: validates a
/// real NISAR HDF5 fixture WITHOUT establishing process isolation first. When
/// isolation is absent the validator must fail closed with
/// `NISAR_HDF5_VALIDATION_UNAVAILABLE`.
#[cfg(test)]
pub(crate) fn validate_probe_without_isolation(
    path: &Path,
    scope: FrequencyScope,
) -> Result<(), ArtifactValidationError> {
    validate_with_registry(&ArtifactValidatorRegistry::standard(), path, scope)
}

fn validate_v2(path: &Path, scope: FrequencyScope) -> Result<(), ArtifactValidationError> {
    let _test_guard = insar_download::hdf5_plugin_isolation::HDF5_TEST_LOCK
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    insar_download::establish_hdf5_plugin_isolation()
        .expect("app-owned HDF5 plugin isolation is required by the validator");
    let asset = resolved_asset();
    ArtifactValidatorRegistry::standard().validate_for_test(
        v2_binding(),
        path,
        &asset,
        validation_context_v2(scope),
    )
}

fn validation_context(scope: FrequencyScope) -> ArtifactValidationContext {
    let (frequency_a, frequency_b) = match scope {
        FrequencyScope::AOnly => (Some("HH"), None),
        FrequencyScope::BOnly => (None, Some("VV")),
        FrequencyScope::Dual => (Some("HH+HV"), Some("VV")),
    };
    NisarRslcValidationContext::from_frozen_metadata(
        super::nisar_hdf5_support::GRANULE_ID,
        frequency_a,
        frequency_b,
    )
    .expect("fixture context follows the frozen NISAR value domain")
    .into()
}

fn validation_context_v2(scope: FrequencyScope) -> ArtifactValidationContext {
    let (frequency_a, frequency_b) = match scope {
        FrequencyScope::AOnly => (Some("HH"), None),
        FrequencyScope::BOnly => (None, Some("VV")),
        FrequencyScope::Dual => (Some("HH+HV"), Some("VV")),
    };
    NisarRslcValidationContext::from_frozen_product_profile(
        super::nisar_hdf5_support::GRANULE_ID,
        frequency_a,
        frequency_b,
        "P05023",
        P05023_PRODUCT_SPECIFICATION_VERSION,
        insar_provider::catalog::NISAR_PRODUCT_SPECIFICATION_PROFILE_REGISTRY_VERSION,
    )
    .expect("fixture context follows the frozen P05023 product profile")
    .into()
}

fn resolved_asset() -> ResolvedAsset {
    ResolvedAsset::new(
        "https://nisar.asf.earthdatacloud.nasa.gov/fixture.h5",
        ResolvedMethod::Get,
    )
    .expect("valid synthetic resolved NISAR asset")
}

fn assert_issue(
    result: Result<(), ArtifactValidationError>,
    kind: ArtifactValidationErrorKind,
    issue: ArtifactValidationIssue,
) {
    let error = result.expect_err("malformed fixture must fail structural validation");
    assert_eq!(error.kind(), kind);
    assert_eq!(error.issue(), issue);
    assert_eq!(
        error.code(),
        match kind {
            ArtifactValidationErrorKind::Unavailable => "NISAR_HDF5_VALIDATION_UNAVAILABLE",
            ArtifactValidationErrorKind::StructureInvalid => "NISAR_HDF5_STRUCTURE_INVALID",
            ArtifactValidationErrorKind::LocalReadFailed => "NISAR_HDF5_LOCAL_READ_FAILED",
        }
    );
}

#[test]
fn valid_a_only_b_only_and_dual_frequency_rslc_files_are_accepted() {
    let fixtures = FixtureDirectory::new("valid-frequency-scopes");
    for (label, scope) in [
        ("a-only", FrequencyScope::AOnly),
        ("b-only", FrequencyScope::BOnly),
        ("dual", FrequencyScope::Dual),
    ] {
        let path = fixtures.file(label);
        create_valid_fixture(&path, scope);
        let before = fs::read(&path).expect("read fixture before validation");
        validate(&path, scope).unwrap_or_else(|error| panic!("{label} failed: {error}"));
        assert_eq!(
            fs::read(&path).expect("read fixture after validation"),
            before,
            "validator must not modify {label}"
        );
    }

    let allowed_filters = fixtures.file("allowed-filter-pipeline");
    create_allowed_filter_fixture(&allowed_filters);
    let before = fs::read(&allowed_filters).expect("read filtered fixture before validation");
    validate(&allowed_filters, FrequencyScope::AOnly)
        .expect("shuffle, deflate, and Fletcher32 are the complete allowed filter set");
    assert_eq!(
        fs::read(&allowed_filters).expect("read filtered fixture after validation"),
        before,
        "validator must not modify filtered fixtures"
    );
}

/// L3 gate (2026-10-19): a NISAR product whose path is longer than the
/// Windows MAX_PATH limit (260 characters) passed every byte check and was then
/// refused as NISAR_HDF5_STRUCTURE_INVALID because the HDF5 library could not
/// open it. Its structure is validated like that of any other path.
#[test]
fn a_valid_product_under_a_path_longer_than_max_path_is_accepted() {
    let fixtures = FixtureDirectory::new("long-path");
    let short = fixtures.file("short");
    create_valid_fixture(&short, FrequencyScope::AOnly);
    let mut directory = short.parent().unwrap().to_path_buf();
    while directory.as_os_str().len() < 240 {
        directory.push("d".repeat(40));
    }
    fs::create_dir_all(&directory).unwrap();
    let long = directory.join(format!("{}.h5", super::nisar_hdf5_support::GRANULE_ID));
    assert!(long.as_os_str().len() > 260, "{}", long.display());
    fs::rename(&short, &long).unwrap();
    let before = fs::read(&long).unwrap();
    validate(&long, FrequencyScope::AOnly)
        .unwrap_or_else(|error| panic!("a long path failed validation: {error}"));
    assert_eq!(
        fs::read(&long).unwrap(),
        before,
        "validator must not modify the file"
    );
}

/// L3 gate (2026-10-19): when the operating system refuses to open the
/// candidate, its bytes were never judged. That is a local read failure a retry
/// may resolve, not NISAR_HDF5_STRUCTURE_INVALID.
#[test]
fn a_candidate_the_system_cannot_open_is_a_retryable_local_read_failure() {
    let fixtures = FixtureDirectory::new("local-read");
    let missing = fixtures.file("missing");
    let error = validate(&missing, FrequencyScope::AOnly).expect_err("a missing candidate");
    assert_eq!(error.kind(), ArtifactValidationErrorKind::LocalReadFailed);
    assert_eq!(error.issue(), ArtifactValidationIssue::CandidateIo);
    assert_eq!(error.code(), "NISAR_HDF5_LOCAL_READ_FAILED");
    assert!(error.retryable());
    assert!(error.reason_zh().contains("可稍后重试"));

    // HDF5 refusing the open at the operating-system level is classified the
    // same way; HDF5 failing on the contents remains a structure verdict.
    let refused = hdf5::File::open(&missing).expect_err("HDF5 cannot open a missing file");
    let refused = crate::nisar_hdf5_validation::hdf5_open_error(&refused);
    assert_eq!(refused.kind(), ArtifactValidationErrorKind::LocalReadFailed);
    assert_eq!(refused.issue(), ArtifactValidationIssue::Hdf5Open);
    let signature_only = fixtures.file("signature-only");
    fs::write(&signature_only, b"\x89HDF\r\n\x1a\n").unwrap();
    let unreadable =
        hdf5::File::open(&signature_only).expect_err("HDF5 cannot read a signature-only file");
    let unreadable = crate::nisar_hdf5_validation::hdf5_open_error(&unreadable);
    assert_eq!(
        unreadable.kind(),
        ArtifactValidationErrorKind::StructureInvalid
    );
    assert!(!unreadable.retryable());
}

#[test]
fn validator_versions_are_binding_selected_and_keep_v1_frozen_at_spec_1_2_1() {
    assert_eq!(NISAR_RSLC_STRUCTURE_VALIDATOR_VERSION, 1);
    assert_eq!(NISAR_RSLC_STRUCTURE_VALIDATOR_V2_VERSION, 2);

    let fixtures = FixtureDirectory::new("validator-versions");
    let legacy = fixtures.file("legacy-1-2-1");
    create_valid_fixture_for_profile(
        &legacy,
        FrequencyScope::AOnly,
        super::nisar_hdf5_support::GRANULE_ID,
        LEGACY_PRODUCT_SPECIFICATION_VERSION,
    );
    validate(&legacy, FrequencyScope::AOnly).expect("@1 accepts only its frozen 1.2.1 profile");
    assert_issue(
        validate_v2(&legacy, FrequencyScope::AOnly),
        ArtifactValidationErrorKind::StructureInvalid,
        ArtifactValidationIssue::IdentificationMismatch,
    );

    let p05023 = fixtures.file("p05023-1-5-0");
    create_valid_fixture_for_profile(
        &p05023,
        FrequencyScope::AOnly,
        super::nisar_hdf5_support::GRANULE_ID,
        P05023_PRODUCT_SPECIFICATION_VERSION,
    );
    validate_v2(&p05023, FrequencyScope::AOnly)
        .expect("@2 accepts the P05023 product specification 1.5.0 profile");
    assert_issue(
        validate(&p05023, FrequencyScope::AOnly),
        ArtifactValidationErrorKind::StructureInvalid,
        ArtifactValidationIssue::IdentificationMismatch,
    );
}

#[test]
fn v2_rejects_frozen_granule_frequency_polarization_and_specification_drift() {
    let fixtures = FixtureDirectory::new("v2-identity-drift");
    for (label, mutation, issue) in [
        (
            "wrong-granule",
            FixtureMutation::WrongGranuleId,
            ArtifactValidationIssue::IdentificationMismatch,
        ),
        (
            "wrong-frequency",
            FixtureMutation::WrongFrequencyList,
            ArtifactValidationIssue::FrequencyMismatch,
        ),
        (
            "wrong-polarization",
            FixtureMutation::WrongPolarizationList,
            ArtifactValidationIssue::PolarizationMismatch,
        ),
        (
            "wrong-product-specification-version",
            FixtureMutation::WrongProductSpecificationVersion,
            ArtifactValidationIssue::IdentificationMismatch,
        ),
        (
            "missing-composite-release-id",
            FixtureMutation::MissingCompositeReleaseId,
            ArtifactValidationIssue::IdentificationMismatch,
        ),
        (
            "wrong-composite-release-id",
            FixtureMutation::WrongCompositeReleaseId,
            ArtifactValidationIssue::IdentificationMismatch,
        ),
    ] {
        let path = fixtures.file(label);
        create_valid_fixture_for_profile(
            &path,
            FrequencyScope::AOnly,
            super::nisar_hdf5_support::GRANULE_ID,
            P05023_PRODUCT_SPECIFICATION_VERSION,
        );
        apply_mutation_to_fixture(&path, mutation);
        assert_issue(
            validate_v2(&path, FrequencyScope::AOnly),
            ArtifactValidationErrorKind::StructureInvalid,
            issue,
        );
    }
}

#[test]
fn runtime_and_link_time_hdf5_versions_are_the_reviewed_static_version() {
    let expected = hdf5_sys::Version {
        major: 2,
        minor: 1,
        micro: 0,
    };
    assert_eq!(hdf5_sys::HDF5_VERSION, expected);
    assert_eq!(hdf5::library_version(), (2, 1, 0));
}

#[test]
fn signature_and_read_only_hdf5_open_are_required() {
    let fixtures = FixtureDirectory::new("signature");
    let wrong_signature = fixtures.file("wrong-signature");
    fs::write(&wrong_signature, b"not an HDF5 file").expect("write malformed signature fixture");
    assert_issue(
        validate(&wrong_signature, FrequencyScope::AOnly),
        ArtifactValidationErrorKind::StructureInvalid,
        ArtifactValidationIssue::InvalidSignature,
    );

    let signature_only = fixtures.file("signature-only");
    fs::write(&signature_only, b"\x89HDF\r\n\x1a\n").expect("write signature-only fixture");
    assert_issue(
        validate(&signature_only, FrequencyScope::AOnly),
        ArtifactValidationErrorKind::StructureInvalid,
        ArtifactValidationIssue::Hdf5Open,
    );
}

#[test]
fn required_rslc_identity_and_legacy_slc_rejection_are_closed() {
    let fixtures = FixtureDirectory::new("identity");
    for (label, mutation, issue) in [
        (
            "missing-swaths",
            FixtureMutation::MissingSwaths,
            ArtifactValidationIssue::RequiredObjectMissing,
        ),
        (
            "wrong-mission",
            FixtureMutation::WrongMissionId,
            ArtifactValidationIssue::IdentificationMismatch,
        ),
        (
            "wrong-product-type",
            FixtureMutation::WrongProductType,
            ArtifactValidationIssue::IdentificationMismatch,
        ),
        (
            "wrong-product-level",
            FixtureMutation::WrongProductLevel,
            ArtifactValidationIssue::IdentificationMismatch,
        ),
        (
            "wrong-radar-band",
            FixtureMutation::WrongRadarBand,
            ArtifactValidationIssue::IdentificationMismatch,
        ),
        (
            "wrong-product-specification-version",
            FixtureMutation::WrongProductSpecificationVersion,
            ArtifactValidationIssue::IdentificationMismatch,
        ),
        (
            "wrong-granule",
            FixtureMutation::WrongGranuleId,
            ArtifactValidationIssue::IdentificationMismatch,
        ),
        (
            "legacy-slc",
            FixtureMutation::LegacySlc,
            ArtifactValidationIssue::IdentificationMismatch,
        ),
        (
            "s-band-group",
            FixtureMutation::Ssar,
            ArtifactValidationIssue::IdentificationMismatch,
        ),
    ] {
        let path = fixtures.file(label);
        create_mutated_fixture(&path, mutation);
        assert_issue(
            validate(&path, FrequencyScope::AOnly),
            ArtifactValidationErrorKind::StructureInvalid,
            issue,
        );
    }
}

#[test]
fn frozen_frequency_polarization_and_cfloat32_image_contracts_are_exact() {
    let fixtures = FixtureDirectory::new("science-identity");
    for (label, mutation, issue) in [
        (
            "wrong-frequency",
            FixtureMutation::WrongFrequencyList,
            ArtifactValidationIssue::FrequencyMismatch,
        ),
        (
            "wrong-polarization",
            FixtureMutation::WrongPolarizationList,
            ArtifactValidationIssue::PolarizationMismatch,
        ),
        (
            "wrong-image-type",
            FixtureMutation::WrongImageType,
            ArtifactValidationIssue::ImageDatasetInvalid,
        ),
        (
            "wrong-image-shape",
            FixtureMutation::WrongImageShape,
            ArtifactValidationIssue::ImageDatasetInvalid,
        ),
    ] {
        let path = fixtures.file(label);
        create_mutated_fixture(&path, mutation);
        assert_issue(
            validate(&path, FrequencyScope::AOnly),
            ArtifactValidationErrorKind::StructureInvalid,
            issue,
        );
    }
}

#[test]
fn bounding_polygon_and_geolocation_metadata_are_bounded_and_typed() {
    let fixtures = FixtureDirectory::new("bounded-geolocation");
    for (label, mutation, issue) in [
        (
            "empty-polygon",
            FixtureMutation::EmptyBoundingPolygon,
            ArtifactValidationIssue::BoundingPolygonInvalid,
        ),
        (
            "wrong-coordinate-shape",
            FixtureMutation::WrongGeolocationShape,
            ArtifactValidationIssue::GeolocationGridInvalid,
        ),
        (
            "metadata-array-limit",
            FixtureMutation::MetadataArrayLimit,
            ArtifactValidationIssue::FrequencyMismatch,
        ),
        (
            "metadata-string-budget",
            FixtureMutation::MetadataStringBudget,
            ArtifactValidationIssue::TraversalLimit,
        ),
    ] {
        let path = fixtures.file(label);
        create_mutated_fixture(&path, mutation);
        assert_issue(
            validate(&path, FrequencyScope::AOnly),
            ArtifactValidationErrorKind::StructureInvalid,
            issue,
        );
    }
}

#[test]
fn metadata_read_budgets_reject_resizable_extents_and_oversized_chunks_before_read_1d() {
    let fixtures = FixtureDirectory::new("metadata-read-budget");
    for (label, mutation) in [
        (
            "unlimited-max-extent",
            FixtureMutation::MetadataUnlimitedMaxExtent,
        ),
        (
            "larger-finite-max-extent",
            FixtureMutation::MetadataLargerFiniteMaxExtent,
        ),
        (
            "oversized-compressed-chunk",
            FixtureMutation::MetadataOversizedCompressedChunk,
        ),
        (
            "filtered-stored-chunk",
            FixtureMutation::MetadataFilteredChunk,
        ),
    ] {
        let path = fixtures.file(label);
        create_mutated_fixture(&path, mutation);
        let reads_before = read_1d_call_count_for_test();
        assert_issue(
            validate(&path, FrequencyScope::AOnly),
            ArtifactValidationErrorKind::StructureInvalid,
            ArtifactValidationIssue::DatasetReadBudget,
        );
        assert_eq!(
            read_1d_call_count_for_test(),
            reads_before,
            "{label} must be rejected before read_1d"
        );
    }
}

#[test]
fn every_link_must_be_hard_and_object_aliases_are_rejected() {
    let fixtures = FixtureDirectory::new("links");
    for (label, mutation, issue) in [
        (
            "soft-link",
            FixtureMutation::SoftLink,
            ArtifactValidationIssue::LinkNotHard,
        ),
        (
            "external-link",
            FixtureMutation::ExternalLink,
            ArtifactValidationIssue::LinkNotHard,
        ),
        (
            "hard-alias",
            FixtureMutation::HardLinkAlias,
            ArtifactValidationIssue::ObjectAliasOrCycle,
        ),
    ] {
        let path = fixtures.file(label);
        create_mutated_fixture(&path, mutation);
        assert_issue(
            validate(&path, FrequencyScope::AOnly),
            ArtifactValidationErrorKind::StructureInvalid,
            issue,
        );
    }
}

#[test]
fn vds_external_storage_and_unknown_filters_are_rejected_before_value_reads() {
    let fixtures = FixtureDirectory::new("dataset-storage");
    for (label, mutation, issue) in [
        (
            "unknown-filter",
            FixtureMutation::UnknownFilter,
            ArtifactValidationIssue::DatasetFilter,
        ),
        (
            "filter-count-limit",
            FixtureMutation::FilterCountLimit,
            ArtifactValidationIssue::DatasetFilter,
        ),
        (
            "filter-client-data-limit",
            FixtureMutation::FilterClientDataLimit,
            ArtifactValidationIssue::DatasetFilter,
        ),
        (
            "deflate-level",
            FixtureMutation::DeflateLevel,
            ArtifactValidationIssue::DatasetFilter,
        ),
        (
            "fletcher-client-data",
            FixtureMutation::FletcherClientData,
            ArtifactValidationIssue::DatasetFilter,
        ),
        (
            "external-storage",
            FixtureMutation::ExternalStorage,
            ArtifactValidationIssue::DatasetExternalStorage,
        ),
        (
            "virtual-dataset",
            FixtureMutation::VirtualDataset,
            ArtifactValidationIssue::DatasetVirtualLayout,
        ),
    ] {
        let path = fixtures.file(label);
        create_mutated_fixture(&path, mutation);
        assert_issue(
            validate(&path, FrequencyScope::AOnly),
            ArtifactValidationErrorKind::StructureInvalid,
            issue,
        );
    }
}

#[test]
fn traversal_member_name_attribute_and_depth_limits_fail_closed() {
    let fixtures = FixtureDirectory::new("traversal-limits");
    for (label, mutation, issue) in [
        (
            "direct-members",
            FixtureMutation::DirectMemberLimit,
            ArtifactValidationIssue::TraversalLimit,
        ),
        (
            "long-name",
            FixtureMutation::LinkNameLimit,
            ArtifactValidationIssue::LinkNameLimit,
        ),
        (
            "invalid-utf8-name",
            FixtureMutation::InvalidUtf8LinkName,
            ArtifactValidationIssue::LinkNameLimit,
        ),
        (
            "attributes",
            FixtureMutation::AttributeLimit,
            ArtifactValidationIssue::AttributeLimit,
        ),
        (
            "depth",
            FixtureMutation::DepthLimit,
            ArtifactValidationIssue::TraversalLimit,
        ),
    ] {
        let path = fixtures.file(label);
        create_mutated_fixture(&path, mutation);
        assert_issue(
            validate(&path, FrequencyScope::AOnly),
            ArtifactValidationErrorKind::StructureInvalid,
            issue,
        );
    }
}

#[test]
fn dataset_rank_dimension_and_deadline_limits_fail_closed() {
    let fixtures = FixtureDirectory::new("dataset-limits");
    for (label, mutation) in [
        ("rank", FixtureMutation::RankLimit),
        ("dimension", FixtureMutation::DimensionLimit),
    ] {
        let path = fixtures.file(label);
        create_mutated_fixture(&path, mutation);
        assert_issue(
            validate(&path, FrequencyScope::AOnly),
            ArtifactValidationErrorKind::StructureInvalid,
            ArtifactValidationIssue::DatasetShape,
        );
    }

    let deadline = fixtures.file("deadline");
    create_valid_fixture(&deadline, FrequencyScope::AOnly);
    let registry = ArtifactValidatorRegistry::nisar_with_deadline_for_test(Duration::ZERO);
    assert_issue(
        validate_with_registry(&registry, &deadline, FrequencyScope::AOnly),
        ArtifactValidationErrorKind::StructureInvalid,
        ArtifactValidationIssue::DeadlineExceeded,
    );
}

#[test]
fn huge_sparse_cfloat32_image_is_validated_without_materialization() {
    let fixtures = FixtureDirectory::new("huge-logical-image");
    let path = fixtures.file("huge");
    create_huge_logical_fixture(&path);

    let physical_bytes = fs::metadata(&path)
        .expect("huge logical fixture metadata")
        .len();
    assert!(
        physical_bytes < 2 * 1024 * 1024,
        "unwritten logical raster must stay sparse, got {physical_bytes} physical bytes"
    );
    validate(&path, FrequencyScope::AOnly)
        .expect("metadata-only validation must not materialize the logical raster");
}

#[test]
fn validation_fails_closed_when_hdf5_plugin_isolation_is_absent() {
    let _guard = insar_download::hdf5_plugin_isolation::HDF5_TEST_LOCK
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let fixtures = FixtureDirectory::new("no-isolation");
    let path = fixtures.file("valid");
    create_valid_fixture(&path, FrequencyScope::Dual);
    insar_download::hdf5_plugin_isolation::crt_env::remove("HDF5_PLUGIN_PRELOAD");
    insar_download::hdf5_plugin_isolation::reenable_hdf5_plugin_loading_for_test();
    // If HDF5 was initialised with an inherited HDF5_PLUGIN_PRELOAD=::
    // sentinel the library permanently locks plugin loading off and no
    // in-process negative control is possible; detect that from the live mask
    // and skip instead of failing the gate.
    if insar_download::hdf5_plugin_loading_disabled() {
        insar_download::establish_hdf5_plugin_isolation()
            .expect("restore isolation for the remaining suite");
        return;
    }
    let error = validate_with_registry(
        &ArtifactValidatorRegistry::standard(),
        &path,
        FrequencyScope::Dual,
    )
    .expect_err("validator must fail closed without isolation");
    assert_eq!(error.kind(), ArtifactValidationErrorKind::Unavailable);
    assert_eq!(error.code(), "NISAR_HDF5_VALIDATION_UNAVAILABLE");
    insar_download::establish_hdf5_plugin_isolation()
        .expect("restore isolation for the remaining suite");
    // validate() re-acquires the test lock, which this test already holds, so
    // use the lock-free registry path to prove the same file now validates.
    validate_with_registry(
        &ArtifactValidatorRegistry::standard(),
        &path,
        FrequencyScope::Dual,
    )
    .expect("validator works once isolation is restored");
}

#[test]
fn isolation_established_without_env_sentinel_and_ignores_hostile_plugin_path() {
    let _guard = insar_download::hdf5_plugin_isolation::HDF5_TEST_LOCK
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    insar_download::hdf5_plugin_isolation::crt_env::remove("HDF5_PLUGIN_PRELOAD");
    let hostile =
        std::env::temp_dir().join(format!("insar-hdf5-suite-hostile-{}", std::process::id()));
    std::fs::create_dir_all(&hostile).unwrap();
    std::fs::write(hostile.join("hdf5_plugin.dll"), b"not a plugin").unwrap();
    assert!(insar_download::hdf5_plugin_isolation::crt_env::set(
        "HDF5_PLUGIN_PATH",
        &hostile.to_string_lossy(),
    ));

    insar_download::establish_hdf5_plugin_isolation()
        .expect("app-owned isolation succeeds without parent env injection");
    assert!(insar_download::hdf5_plugin_loading_disabled());

    let fixtures = FixtureDirectory::new("isolation-positive");
    let path = fixtures.file("valid");
    create_valid_fixture(&path, FrequencyScope::Dual);
    // validate() re-acquires the test lock, which this test already holds, so
    // use the lock-free registry path.
    validate_with_registry(
        &ArtifactValidatorRegistry::standard(),
        &path,
        FrequencyScope::Dual,
    )
    .expect("validation works with isolation and hostile path");

    let _ = std::fs::remove_dir_all(&hostile);
}
