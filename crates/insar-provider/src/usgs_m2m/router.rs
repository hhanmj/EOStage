use std::sync::Arc;

use insar_core::ProviderNetworkPolicy;

use crate::catalog::{LANDSAT_C2_L2_LOGICAL_DATASET, LANDSAT_C2_L2_PROVIDER_ID};

use super::discovery::{
    UsgsM2mDatasetDescriptor, UsgsM2mDatasetDiscoveryClient, UsgsM2mDatasetDiscoveryRequest,
    UsgsM2mDatasetDiscoveryResponse,
};
use super::error::{UsgsM2mEndpointClass, UsgsM2mError, UsgsM2mErrorCode};
use super::session::USGS_M2M_SESSION_PROFILE_ID;

pub const USGS_M2M_DISCOVERY_ROUTE_ID: &str = "usgs.eros.m2m.landsat.c2.l2.discovery.v1";
pub const USGS_M2M_DATASET_DISCOVERY_CAPABILITY: &str = "dataset_discovery";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct UsgsM2mLiveDatasetIdentity {
    dataset_id: &'static str,
    dataset_alias: &'static str,
    collection_name: &'static str,
    dataset_category_name: &'static str,
    data_owner: &'static str,
    catalogs: &'static [&'static str],
}

impl UsgsM2mLiveDatasetIdentity {
    pub const fn dataset_id(self) -> &'static str {
        self.dataset_id
    }

    pub const fn dataset_alias(self) -> &'static str {
        self.dataset_alias
    }

    pub const fn collection_name(self) -> &'static str {
        self.collection_name
    }

    pub const fn dataset_category_name(self) -> &'static str {
        self.dataset_category_name
    }

    pub const fn data_owner(self) -> &'static str {
        self.data_owner
    }

    pub const fn catalogs(self) -> &'static [&'static str] {
        self.catalogs
    }

    pub fn matches_descriptor(self, descriptor: &UsgsM2mDatasetDescriptor) -> bool {
        descriptor.identity().dataset_id() == self.dataset_id
            && descriptor.identity().upstream_dataset_identity() == self.dataset_alias
            && descriptor.collection_name() == self.collection_name
            && descriptor.dataset_category_name() == self.dataset_category_name
            && descriptor.data_owner() == self.data_owner
            && descriptor.catalogs().len() == self.catalogs.len()
            && self.catalogs.iter().all(|expected| {
                descriptor
                    .catalogs()
                    .iter()
                    .any(|actual| actual == expected)
            })
    }
}

pub const USGS_M2M_LANDSAT_C2_L2_LIVE_IDENTITY: Option<UsgsM2mLiveDatasetIdentity> =
    Some(UsgsM2mLiveDatasetIdentity {
        dataset_id: "5e83d14f2fc39685",
        dataset_alias: "landsat_ot_c2_l2",
        collection_name: "Landsat 8-9 OLI/TIRS C2 L2",
        dataset_category_name: "Landsat Collection 2 Level-2",
        data_owner: "LSAA",
        catalogs: &["EE"],
    });

pub const USGS_M2M_LANDSAT_TM_C2_L2_LIVE_IDENTITY: Option<UsgsM2mLiveDatasetIdentity> =
    Some(UsgsM2mLiveDatasetIdentity {
        dataset_id: "5e83d14f4e24ef5c",
        dataset_alias: "landsat_tm_c2_l2",
        collection_name: "Landsat 4-5 TM C2 L2",
        dataset_category_name: "Landsat Collection 2 Level-2",
        data_owner: "LSAA",
        catalogs: &["EE"],
    });

pub const USGS_M2M_LANDSAT_ETM_C2_L2_LIVE_IDENTITY: Option<UsgsM2mLiveDatasetIdentity> =
    Some(UsgsM2mLiveDatasetIdentity {
        dataset_id: "5e83d14fb8e96503",
        dataset_alias: "landsat_etm_c2_l2",
        collection_name: "Landsat 7 ETM+ C2 L2",
        dataset_category_name: "Landsat Collection 2 Level-2",
        data_owner: "LSAA",
        catalogs: &["EE"],
    });

pub fn live_identity_for_alias(alias: &str) -> Option<UsgsM2mLiveDatasetIdentity> {
    match alias {
        "landsat_tm_c2_l2" => USGS_M2M_LANDSAT_TM_C2_L2_LIVE_IDENTITY,
        "landsat_etm_c2_l2" => USGS_M2M_LANDSAT_ETM_C2_L2_LIVE_IDENTITY,
        "landsat_ot_c2_l2" => USGS_M2M_LANDSAT_C2_L2_LIVE_IDENTITY,
        _ => None,
    }
}

pub const USGS_M2M_LANDSAT_C2_L2_UPSTREAM_IDENTITY: Option<&str> =
    match USGS_M2M_LANDSAT_C2_L2_LIVE_IDENTITY {
        Some(identity) => Some(identity.dataset_alias()),
        None => None,
    };

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct UsgsM2mSceneSearchAcceptance {
    acceptance_id: &'static str,
    dataset_identity: UsgsM2mLiveDatasetIdentity,
}

impl UsgsM2mSceneSearchAcceptance {
    pub const fn acceptance_id(self) -> &'static str {
        self.acceptance_id
    }

    pub const fn dataset_identity(self) -> UsgsM2mLiveDatasetIdentity {
        self.dataset_identity
    }
}

pub const USGS_M2M_LANDSAT_C2_L2_SCENE_SEARCH_ACCEPTANCE: Option<UsgsM2mSceneSearchAcceptance> =
    match USGS_M2M_LANDSAT_C2_L2_LIVE_IDENTITY {
        Some(dataset_identity) => Some(UsgsM2mSceneSearchAcceptance {
            acceptance_id: "LANDSAT-P4A-2A-LIVE-2026-07-29",
            dataset_identity,
        }),
        None => None,
    };

pub const fn landsat_scene_search_live_accepted() -> bool {
    USGS_M2M_LANDSAT_C2_L2_SCENE_SEARCH_ACCEPTANCE.is_some()
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum UsgsM2mCapability {
    DatasetDiscovery,
    RuntimeSession,
    NetworkPolicy,
}

impl UsgsM2mCapability {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::DatasetDiscovery => USGS_M2M_DATASET_DISCOVERY_CAPABILITY,
            Self::RuntimeSession => "runtime_session",
            Self::NetworkPolicy => "network_policy",
        }
    }
}

const IMPLEMENTED_CAPABILITIES: &[UsgsM2mCapability] = &[
    UsgsM2mCapability::DatasetDiscovery,
    UsgsM2mCapability::RuntimeSession,
    UsgsM2mCapability::NetworkPolicy,
];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum UsgsM2mDiscoveryState {
    OfflineImplemented,
    LiveAccepted,
}

impl UsgsM2mDiscoveryState {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::OfflineImplemented => "implemented_not_live_accepted",
            Self::LiveAccepted => "live_accepted",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum UsgsM2mDiscoveryUse {
    OfflineValidation,
    Production,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct UsgsM2mDatasetDiscoveryRouteRequest {
    logical_dataset: String,
    provider_id: String,
    session_profile_id: String,
    capability: String,
}

impl UsgsM2mDatasetDiscoveryRouteRequest {
    pub fn landsat_c2_l2() -> Self {
        Self::from_parts(
            LANDSAT_C2_L2_LOGICAL_DATASET,
            LANDSAT_C2_L2_PROVIDER_ID,
            USGS_M2M_SESSION_PROFILE_ID,
            USGS_M2M_DATASET_DISCOVERY_CAPABILITY,
        )
    }

    pub fn from_parts(
        logical_dataset: impl Into<String>,
        provider_id: impl Into<String>,
        session_profile_id: impl Into<String>,
        capability: impl Into<String>,
    ) -> Self {
        Self {
            logical_dataset: logical_dataset.into(),
            provider_id: provider_id.into(),
            session_profile_id: session_profile_id.into(),
            capability: capability.into(),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct UsgsM2mDatasetDiscoveryRoute {
    route_id: &'static str,
    logical_dataset: &'static str,
    provider_id: &'static str,
    session_profile_id: &'static str,
    capability: &'static str,
    live_identity: Option<UsgsM2mLiveDatasetIdentity>,
}

impl UsgsM2mDatasetDiscoveryRoute {
    pub const fn route_id(self) -> &'static str {
        self.route_id
    }

    pub const fn logical_dataset(self) -> &'static str {
        self.logical_dataset
    }

    pub const fn provider_id(self) -> &'static str {
        self.provider_id
    }

    pub const fn session_profile_id(self) -> &'static str {
        self.session_profile_id
    }

    pub const fn capability(self) -> &'static str {
        self.capability
    }

    pub const fn state(self) -> UsgsM2mDiscoveryState {
        discovery_state(self.live_identity)
    }

    pub const fn upstream_dataset_identity(self) -> Option<&'static str> {
        match self.live_identity {
            Some(identity) => Some(identity.dataset_alias()),
            None => None,
        }
    }

    pub const fn dataset_id(self) -> Option<&'static str> {
        match self.live_identity {
            Some(identity) => Some(identity.dataset_id()),
            None => None,
        }
    }

    pub const fn live_identity(self) -> Option<UsgsM2mLiveDatasetIdentity> {
        self.live_identity
    }

    pub const fn production_enabled(self) -> bool {
        self.live_identity.is_some()
    }
}

const fn discovery_state(
    live_identity: Option<UsgsM2mLiveDatasetIdentity>,
) -> UsgsM2mDiscoveryState {
    match live_identity {
        Some(_) => UsgsM2mDiscoveryState::LiveAccepted,
        None => UsgsM2mDiscoveryState::OfflineImplemented,
    }
}

const fn discovery_route(
    live_identity: Option<UsgsM2mLiveDatasetIdentity>,
) -> UsgsM2mDatasetDiscoveryRoute {
    UsgsM2mDatasetDiscoveryRoute {
        route_id: USGS_M2M_DISCOVERY_ROUTE_ID,
        logical_dataset: LANDSAT_C2_L2_LOGICAL_DATASET,
        provider_id: LANDSAT_C2_L2_PROVIDER_ID,
        session_profile_id: USGS_M2M_SESSION_PROFILE_ID,
        capability: USGS_M2M_DATASET_DISCOVERY_CAPABILITY,
        live_identity,
    }
}

const DISCOVERY_ROUTE: UsgsM2mDatasetDiscoveryRoute =
    discovery_route(USGS_M2M_LANDSAT_C2_L2_LIVE_IDENTITY);

#[cfg(test)]
const IDENTITY_ABSENT_DISCOVERY_ROUTE: UsgsM2mDatasetDiscoveryRoute = discovery_route(None);

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct UsgsM2mCapabilityReport {
    logical_dataset: &'static str,
    provider_id: &'static str,
    session_profile_id: &'static str,
    capabilities: &'static [UsgsM2mCapability],
    live_identity: Option<UsgsM2mLiveDatasetIdentity>,
    network_policy: ProviderNetworkPolicy,
}

impl UsgsM2mCapabilityReport {
    pub const fn logical_dataset(&self) -> &'static str {
        self.logical_dataset
    }

    pub const fn provider_id(&self) -> &'static str {
        self.provider_id
    }

    pub const fn session_profile_id(&self) -> &'static str {
        self.session_profile_id
    }

    pub const fn capabilities(&self) -> &'static [UsgsM2mCapability] {
        self.capabilities
    }

    pub const fn state(&self) -> UsgsM2mDiscoveryState {
        discovery_state(self.live_identity)
    }

    pub const fn upstream_dataset_identity(&self) -> Option<&'static str> {
        match self.live_identity {
            Some(identity) => Some(identity.dataset_alias()),
            None => None,
        }
    }

    pub const fn dataset_id(&self) -> Option<&'static str> {
        match self.live_identity {
            Some(identity) => Some(identity.dataset_id()),
            None => None,
        }
    }

    pub const fn live_identity(&self) -> Option<UsgsM2mLiveDatasetIdentity> {
        self.live_identity
    }

    pub const fn production_enabled(&self) -> bool {
        self.live_identity.is_some()
    }

    pub const fn network_policy(&self) -> ProviderNetworkPolicy {
        self.network_policy
    }
}

trait DatasetDiscoveryPort: Send + Sync {
    fn network_policy(&self) -> ProviderNetworkPolicy;

    fn discover(
        &self,
        request: UsgsM2mDatasetDiscoveryRequest,
        cancelled: &(dyn Fn() -> bool + Sync),
    ) -> Result<UsgsM2mDatasetDiscoveryResponse, UsgsM2mError>;
}

impl DatasetDiscoveryPort for UsgsM2mDatasetDiscoveryClient {
    fn network_policy(&self) -> ProviderNetworkPolicy {
        self.network_policy()
    }

    fn discover(
        &self,
        request: UsgsM2mDatasetDiscoveryRequest,
        cancelled: &(dyn Fn() -> bool + Sync),
    ) -> Result<UsgsM2mDatasetDiscoveryResponse, UsgsM2mError> {
        self.discover(request, cancelled)
    }
}

pub struct UsgsM2mDatasetDiscoveryRouter {
    port: Arc<dyn DatasetDiscoveryPort>,
    route: UsgsM2mDatasetDiscoveryRoute,
    offline_validation_enabled: bool,
}

impl UsgsM2mDatasetDiscoveryRouter {
    pub fn new(client: Arc<UsgsM2mDatasetDiscoveryClient>) -> Self {
        Self {
            port: client,
            route: DISCOVERY_ROUTE,
            offline_validation_enabled: false,
        }
    }

    #[cfg(test)]
    fn from_port(port: Arc<dyn DatasetDiscoveryPort>) -> Self {
        Self {
            port,
            route: IDENTITY_ABSENT_DISCOVERY_ROUTE,
            offline_validation_enabled: true,
        }
    }

    #[cfg(test)]
    fn from_live_port(port: Arc<dyn DatasetDiscoveryPort>) -> Self {
        Self {
            port,
            route: DISCOVERY_ROUTE,
            offline_validation_enabled: true,
        }
    }

    pub fn capability_report(&self) -> UsgsM2mCapabilityReport {
        UsgsM2mCapabilityReport {
            logical_dataset: self.route.logical_dataset,
            provider_id: self.route.provider_id,
            session_profile_id: self.route.session_profile_id,
            capabilities: IMPLEMENTED_CAPABILITIES,
            live_identity: self.route.live_identity,
            network_policy: self.port.network_policy(),
        }
    }

    pub fn resolve(
        &self,
        request: &UsgsM2mDatasetDiscoveryRouteRequest,
        use_case: UsgsM2mDiscoveryUse,
    ) -> Result<UsgsM2mDatasetDiscoveryRoute, UsgsM2mError> {
        if request.logical_dataset != self.route.logical_dataset
            || request.provider_id != self.route.provider_id
            || request.session_profile_id != self.route.session_profile_id
            || request.capability != self.route.capability
        {
            return Err(router_error());
        }
        if matches!(use_case, UsgsM2mDiscoveryUse::OfflineValidation)
            && !self.offline_validation_enabled
        {
            return Err(router_error());
        }
        if matches!(use_case, UsgsM2mDiscoveryUse::Production) && !self.route.production_enabled() {
            return Err(router_error());
        }
        Ok(self.route)
    }

    pub fn discover(
        &self,
        request: &UsgsM2mDatasetDiscoveryRouteRequest,
        use_case: UsgsM2mDiscoveryUse,
        cancelled: &(dyn Fn() -> bool + Sync),
    ) -> Result<UsgsM2mDatasetDiscoveryResponse, UsgsM2mError> {
        let route = self.resolve(request, use_case)?;
        let response = self
            .port
            .discover(UsgsM2mDatasetDiscoveryRequest::landsat_c2_l2(), cancelled)?;
        if matches!(use_case, UsgsM2mDiscoveryUse::Production) {
            let Some(identity) = route.live_identity() else {
                return Err(router_error());
            };
            if !identity.matches_descriptor(response.descriptor()) {
                return Err(UsgsM2mError::new(
                    UsgsM2mErrorCode::DatasetConflict,
                    UsgsM2mEndpointClass::DiscoveryRouter,
                ));
            }
        }
        Ok(response)
    }
}

fn router_error() -> UsgsM2mError {
    UsgsM2mError::new(
        UsgsM2mErrorCode::ConfigInvalid,
        UsgsM2mEndpointClass::DiscoveryRouter,
    )
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicUsize, Ordering};

    use serde_json::json;

    use super::*;
    use crate::usgs_m2m::discovery::select_landsat_c2_l2_descriptor;
    use crate::usgs_m2m::endpoint::SensitiveJson;

    struct FixturePort {
        policy: ProviderNetworkPolicy,
        calls: AtomicUsize,
    }

    impl DatasetDiscoveryPort for FixturePort {
        fn network_policy(&self) -> ProviderNetworkPolicy {
            self.policy
        }

        fn discover(
            &self,
            _request: UsgsM2mDatasetDiscoveryRequest,
            _cancelled: &(dyn Fn() -> bool + Sync),
        ) -> Result<UsgsM2mDatasetDiscoveryResponse, UsgsM2mError> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            Err(UsgsM2mError::new(
                UsgsM2mErrorCode::DatasetNotFound,
                UsgsM2mEndpointClass::DatasetDiscovery,
            ))
        }
    }

    struct ResponsePort {
        policy: ProviderNetworkPolicy,
        calls: AtomicUsize,
        response: UsgsM2mDatasetDiscoveryResponse,
    }

    impl DatasetDiscoveryPort for ResponsePort {
        fn network_policy(&self) -> ProviderNetworkPolicy {
            self.policy
        }

        fn discover(
            &self,
            _request: UsgsM2mDatasetDiscoveryRequest,
            _cancelled: &(dyn Fn() -> bool + Sync),
        ) -> Result<UsgsM2mDatasetDiscoveryResponse, UsgsM2mError> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            Ok(self.response.clone())
        }
    }

    fn live_shaped_response(
        dataset_id: &str,
        dataset_alias: &str,
    ) -> UsgsM2mDatasetDiscoveryResponse {
        let identity = USGS_M2M_LANDSAT_C2_L2_LIVE_IDENTITY.expect("live identity is closed");
        select_landsat_c2_l2_descriptor(SensitiveJson::new(json!([{
            "datasetId": dataset_id,
            "datasetAlias": dataset_alias,
            "collectionName": identity.collection_name(),
            "collectionLongName": "Fixture Landsat 8-9 Collection 2 Level-2",
            "datasetCategoryName": identity.dataset_category_name(),
            "dataOwner": identity.data_owner(),
            "catalogs": identity.catalogs(),
        }])))
        .expect("fixture matches the bounded discovery schema")
    }

    #[test]
    fn report_is_discovery_only_offline_implemented_and_production_disabled() {
        let port = Arc::new(FixturePort {
            policy: ProviderNetworkPolicy::direct(),
            calls: AtomicUsize::new(0),
        });
        let router = UsgsM2mDatasetDiscoveryRouter::from_port(port);
        let report = router.capability_report();

        assert_eq!(report.logical_dataset(), LANDSAT_C2_L2_LOGICAL_DATASET);
        assert_eq!(report.provider_id(), LANDSAT_C2_L2_PROVIDER_ID);
        assert_eq!(report.session_profile_id(), USGS_M2M_SESSION_PROFILE_ID);
        assert_eq!(
            report.capabilities(),
            &[
                UsgsM2mCapability::DatasetDiscovery,
                UsgsM2mCapability::RuntimeSession,
                UsgsM2mCapability::NetworkPolicy,
            ]
        );
        assert_eq!(report.state(), UsgsM2mDiscoveryState::OfflineImplemented);
        assert_eq!(report.state().as_str(), "implemented_not_live_accepted");
        assert_eq!(report.live_identity(), None);
        assert_eq!(report.dataset_id(), None);
        assert_eq!(report.upstream_dataset_identity(), None);
        assert!(!report.production_enabled());
        assert_eq!(report.network_policy(), ProviderNetworkPolicy::direct());
    }

    #[test]
    fn exact_offline_route_dispatches_once_but_production_is_disabled() {
        let port = Arc::new(FixturePort {
            policy: ProviderNetworkPolicy::system(),
            calls: AtomicUsize::new(0),
        });
        let router = UsgsM2mDatasetDiscoveryRouter::from_port(port.clone());
        let request = UsgsM2mDatasetDiscoveryRouteRequest::landsat_c2_l2();
        let route = router
            .resolve(&request, UsgsM2mDiscoveryUse::OfflineValidation)
            .unwrap();
        assert_eq!(route.route_id(), USGS_M2M_DISCOVERY_ROUTE_ID);
        assert_eq!(route.state(), UsgsM2mDiscoveryState::OfflineImplemented);
        assert_eq!(route.live_identity(), None);
        assert_eq!(route.dataset_id(), None);
        assert!(!route.production_enabled());
        assert_eq!(route.upstream_dataset_identity(), None);
        assert_eq!(
            router
                .discover(&request, UsgsM2mDiscoveryUse::OfflineValidation, &|| false)
                .unwrap_err()
                .code(),
            UsgsM2mErrorCode::DatasetNotFound
        );
        assert_eq!(port.calls.load(Ordering::SeqCst), 1);
        assert_eq!(
            router
                .resolve(&request, UsgsM2mDiscoveryUse::Production)
                .unwrap_err()
                .code(),
            UsgsM2mErrorCode::ConfigInvalid
        );
        assert_eq!(port.calls.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn live_identity_enables_production_and_dispatches_the_port_exactly_once() {
        let identity = USGS_M2M_LANDSAT_C2_L2_LIVE_IDENTITY.expect("live identity is closed");
        let port = Arc::new(ResponsePort {
            policy: ProviderNetworkPolicy::system(),
            calls: AtomicUsize::new(0),
            response: live_shaped_response(identity.dataset_id(), identity.dataset_alias()),
        });
        let router = UsgsM2mDatasetDiscoveryRouter::from_live_port(port.clone());
        let request = UsgsM2mDatasetDiscoveryRouteRequest::landsat_c2_l2();
        let report = router.capability_report();

        assert_eq!(report.state(), UsgsM2mDiscoveryState::LiveAccepted);
        assert_eq!(report.live_identity(), Some(identity));
        assert!(report.production_enabled());
        let route = router
            .resolve(&request, UsgsM2mDiscoveryUse::Production)
            .unwrap();
        assert_eq!(route.dataset_id(), Some(identity.dataset_id()));
        assert_eq!(
            route.upstream_dataset_identity(),
            Some(identity.dataset_alias())
        );
        let response = router
            .discover(&request, UsgsM2mDiscoveryUse::Production, &|| false)
            .unwrap();
        assert!(identity.matches_descriptor(response.descriptor()));
        assert_eq!(port.calls.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn fixture_identity_cannot_enable_an_identity_absent_production_route() {
        let port = Arc::new(ResponsePort {
            policy: ProviderNetworkPolicy::system(),
            calls: AtomicUsize::new(0),
            response: live_shaped_response("fixture-id", "fixture_l2_alias"),
        });
        let router = UsgsM2mDatasetDiscoveryRouter::from_port(port.clone());
        let error = router
            .discover(
                &UsgsM2mDatasetDiscoveryRouteRequest::landsat_c2_l2(),
                UsgsM2mDiscoveryUse::Production,
                &|| false,
            )
            .unwrap_err();

        assert_eq!(error.code(), UsgsM2mErrorCode::ConfigInvalid);
        assert_eq!(error.endpoint(), UsgsM2mEndpointClass::DiscoveryRouter);
        assert_eq!(port.calls.load(Ordering::SeqCst), 0);
    }

    #[test]
    fn production_identity_conflict_fails_closed_after_one_discovery_call() {
        let identity = USGS_M2M_LANDSAT_C2_L2_LIVE_IDENTITY.expect("live identity is closed");
        let port = Arc::new(ResponsePort {
            policy: ProviderNetworkPolicy::system(),
            calls: AtomicUsize::new(0),
            response: live_shaped_response("conflicting-live-id", identity.dataset_alias()),
        });
        let router = UsgsM2mDatasetDiscoveryRouter::from_live_port(port.clone());
        let error = router
            .discover(
                &UsgsM2mDatasetDiscoveryRouteRequest::landsat_c2_l2(),
                UsgsM2mDiscoveryUse::Production,
                &|| false,
            )
            .unwrap_err();

        assert_eq!(error.code(), UsgsM2mErrorCode::DatasetConflict);
        assert_eq!(error.endpoint(), UsgsM2mEndpointClass::DiscoveryRouter);
        assert_eq!(port.calls.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn every_unknown_route_key_fails_closed_without_calling_a_port() {
        let port = Arc::new(FixturePort {
            policy: ProviderNetworkPolicy::system(),
            calls: AtomicUsize::new(0),
        });
        let router = UsgsM2mDatasetDiscoveryRouter::from_live_port(port.clone());
        for request in [
            UsgsM2mDatasetDiscoveryRouteRequest::from_parts(
                "optical.landsat.c2.l1",
                LANDSAT_C2_L2_PROVIDER_ID,
                USGS_M2M_SESSION_PROFILE_ID,
                USGS_M2M_DATASET_DISCOVERY_CAPABILITY,
            ),
            UsgsM2mDatasetDiscoveryRouteRequest::from_parts(
                LANDSAT_C2_L2_LOGICAL_DATASET,
                "copernicus.dataspace.odata",
                USGS_M2M_SESSION_PROFILE_ID,
                USGS_M2M_DATASET_DISCOVERY_CAPABILITY,
            ),
            UsgsM2mDatasetDiscoveryRouteRequest::from_parts(
                LANDSAT_C2_L2_LOGICAL_DATASET,
                LANDSAT_C2_L2_PROVIDER_ID,
                "earthdata-default",
                USGS_M2M_DATASET_DISCOVERY_CAPABILITY,
            ),
            UsgsM2mDatasetDiscoveryRouteRequest::from_parts(
                LANDSAT_C2_L2_LOGICAL_DATASET,
                LANDSAT_C2_L2_PROVIDER_ID,
                USGS_M2M_SESSION_PROFILE_ID,
                "scene_search",
            ),
        ] {
            assert_eq!(
                router
                    .discover(&request, UsgsM2mDiscoveryUse::Production, &|| false,)
                    .unwrap_err()
                    .code(),
                UsgsM2mErrorCode::ConfigInvalid
            );
        }
        assert_eq!(port.calls.load(Ordering::SeqCst), 0);
    }
}
