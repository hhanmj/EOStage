use std::time::Duration;

use insar_core::ProviderNetworkPolicy;
use insar_provider::usgs_m2m::{
    UsgsM2mApplicationToken, UsgsM2mCredentialProfile, UsgsM2mLoginMaterial, UsgsM2mSession,
    UsgsM2mSessionStatus, USGS_M2M_AUTH_HEADER, USGS_M2M_SESSION_PROFILE_ID,
    USGS_M2M_STABLE_ORIGIN,
};

#[test]
fn application_token_session_contract_is_closed_redacted_and_lazy() {
    let token =
        UsgsM2mApplicationToken::new("fixture-application-token").expect("bounded fixture token");
    let material =
        UsgsM2mLoginMaterial::new("fixture-ers-user", token).expect("bounded login material");
    let debug = format!("{material:?}");

    assert!(!debug.contains("fixture-ers-user"));
    assert!(!debug.contains("fixture-application-token"));
    assert_eq!(
        UsgsM2mCredentialProfile::ApplicationToken.profile_id(),
        USGS_M2M_SESSION_PROFILE_ID
    );
    assert!(
        !UsgsM2mCredentialProfile::ApplicationToken.matches_profile("copernicus-dataspace-default")
    );
    assert!(!UsgsM2mCredentialProfile::ApplicationToken.matches_profile("earthdata-default"));

    let session = UsgsM2mSession::new(ProviderNetworkPolicy::system(), Duration::from_secs(1))
        .expect("fixed production client construction is offline");
    assert_eq!(session.network_policy(), ProviderNetworkPolicy::system());
    assert_eq!(session.status().unwrap(), UsgsM2mSessionStatus::Ready);
    assert!(!format!("{session:?}").contains("fixture-application-token"));
}

#[test]
fn production_constants_freeze_only_the_official_m2m_session_boundary() {
    assert_eq!(
        USGS_M2M_STABLE_ORIGIN,
        "https://m2m.cr.usgs.gov/api/api/json/stable/"
    );
    assert_eq!(USGS_M2M_AUTH_HEADER, "X-Auth-Token");

    assert!(UsgsM2mApplicationToken::new("").is_err());
    assert!(UsgsM2mApplicationToken::new("line\nbreak").is_err());
}
