#[test]
fn production_usgs_module_has_no_cross_provider_or_ambient_credential_path() {
    let sources = [
        include_str!("../src/usgs_m2m/endpoint.rs"),
        include_str!("../src/usgs_m2m/session.rs"),
        include_str!("../src/usgs_m2m/discovery.rs"),
        include_str!("../src/usgs_m2m/router.rs"),
    ];
    let production = sources
        .iter()
        .map(|source| source.split("#[cfg(test)]").next().unwrap())
        .collect::<Vec<_>>()
        .join("\n");

    for forbidden in [
        "std::env",
        "EnvironmentEarthdataCredentialProvider",
        "EarthdataCredential",
        "CdseOAuthSession",
        "CDSE_CREDENTIAL_PROFILE",
        "EARTHDATA_CREDENTIAL_PROFILE",
        ".netrc",
        "Authorization",
        "Bearer",
        "Basic",
        "Cookie",
        "danger_accept_invalid",
    ] {
        assert!(
            !production.contains(forbidden),
            "USGS production module contains forbidden boundary: {forbidden}"
        );
    }

    let session = include_str!("../src/usgs_m2m/session.rs")
        .split("#[cfg(test)]")
        .next()
        .unwrap();
    assert!(!session.contains("Serialize"));
    assert!(production.contains("login-token"));
    assert!(production.contains("X-Auth-Token"));
    assert!(!production.contains("Authorization: Bearer"));
    // The authenticated-session test seam exists only behind the
    // `usgs-m2m-test-session` feature (downstream dev-dependencies). A
    // featureless build must not compile it into the production module:
    // enforce the gate textually. `cargo test --workspace` unifies the
    // insar-download dev-dependency feature into this crate, so the check
    // must not depend on whether the feature is on for this test build.
    let seam_marker = "authenticated_for_tests";
    let seam_is_gated = session
        .lines()
        .any(|line| line.trim() == "#[cfg(feature = \"usgs-m2m-test-session\")]")
        && session.contains(seam_marker);
    assert!(
        seam_is_gated,
        "authenticated_for_tests must stay feature-gated behind usgs-m2m-test-session"
    );
}
