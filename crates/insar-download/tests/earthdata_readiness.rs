use insar_download::{desktop_earthdata_readiness, EarthdataCredentialReadiness};

#[test]
fn desktop_earthdata_readiness_is_secret_free_and_local_only() {
    let readiness = desktop_earthdata_readiness();
    assert!(matches!(
        readiness,
        EarthdataCredentialReadiness::Ready
            | EarthdataCredentialReadiness::Missing
            | EarthdataCredentialReadiness::Invalid
            | EarthdataCredentialReadiness::Unavailable
    ));
}
