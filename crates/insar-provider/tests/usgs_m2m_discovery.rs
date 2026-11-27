use std::sync::Arc;
use std::time::Duration;

use insar_core::ProviderNetworkPolicy;
use insar_provider::usgs_m2m::{
    UsgsM2mDatasetDiscoveryClient, UsgsM2mDatasetDiscoveryRequest, UsgsM2mErrorCode, UsgsM2mSession,
};

#[test]
fn discovery_request_is_closed_and_an_unauthenticated_session_never_reaches_network() {
    let policy = ProviderNetworkPolicy::direct();
    let session = Arc::new(
        UsgsM2mSession::new(policy, Duration::from_secs(1))
            .expect("fixed-origin client construction is lazy"),
    );
    let client = UsgsM2mDatasetDiscoveryClient::new(session);

    assert_eq!(client.network_policy(), policy);
    assert_eq!(
        std::mem::size_of::<UsgsM2mDatasetDiscoveryRequest>(),
        0,
        "the request has no AOI, time, scene, product, download, or pagination state"
    );
    assert_eq!(
        client
            .discover(UsgsM2mDatasetDiscoveryRequest::landsat_c2_l2(), &|| false,)
            .unwrap_err()
            .code(),
        UsgsM2mErrorCode::AuthRequired
    );
}
