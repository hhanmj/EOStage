#[cfg(windows)]
use std::fs::OpenOptions;
use std::io::{Cursor, Read};
#[cfg(windows)]
use std::sync::Arc;
#[cfg(windows)]
use std::time::Duration;

#[cfg(windows)]
use insar_core::ProviderNetworkPolicy;
#[cfg(windows)]
use insar_provider::catalog::{LANDSAT_C2_L2_LOGICAL_DATASET, LANDSAT_C2_L2_PROVIDER_ID};
use insar_provider::usgs_m2m::{UsgsM2mApplicationToken, UsgsM2mLoginMaterial};
#[cfg(windows)]
use insar_provider::usgs_m2m::{
    UsgsM2mCapability, UsgsM2mDatasetDiscoveryClient, UsgsM2mDatasetDiscoveryRequest,
    UsgsM2mDatasetDiscoveryResponse, UsgsM2mDatasetDiscoveryRouteRequest,
    UsgsM2mDatasetDiscoveryRouter, UsgsM2mDiscoveryState, UsgsM2mDiscoveryUse,
    UsgsM2mEndpointClass, UsgsM2mError, UsgsM2mSession, UsgsM2mSessionStatus,
    USGS_M2M_DATASET_DISCOVERY_CAPABILITY, USGS_M2M_DISCOVERY_ROUTE_ID,
    USGS_M2M_LANDSAT_C2_L2_LIVE_IDENTITY, USGS_M2M_SESSION_PROFILE_ID,
};
use zeroize::Zeroizing;

#[cfg(windows)]
const LIVE_CREDENTIAL_PIPE: &str = r"\\.\pipe\insar-studio-usgs-m2m-p4a1-v1";
const MAX_USERNAME_BYTES: usize = 256;
const MAX_APPLICATION_TOKEN_BYTES: usize = 4 * 1024;

#[test]
#[ignore = "requires explicit local USGS credential authorization through the one-time named pipe"]
fn usgs_m2m_dataset_discovery_live_acceptance() {
    #[cfg(not(windows))]
    panic!("USGS_M2M_LIVE_HARNESS_WINDOWS_REQUIRED");

    #[cfg(windows)]
    match run_live_acceptance() {
        Ok(discovery) => {
            eprintln!("USGS_M2M_LIVE_DISCOVERY outcome=completed");
            eprintln!("candidate_count={}", discovery.candidate_count());
            eprintln!("duplicate_count={}", discovery.duplicate_count());
            eprintln!("records_examined={}", discovery.records_examined());
            eprintln!(
                "dataset_id={}",
                discovery.descriptor().identity().dataset_id()
            );
            eprintln!(
                "dataset_alias={}",
                discovery
                    .descriptor()
                    .identity()
                    .upstream_dataset_identity()
            );
            eprintln!("scope_verified=true");
            eprintln!("logout_completed=true");
            eprintln!("production_enabled=false");
        }
        Err(stopped) => {
            eprintln!("USGS_M2M_LIVE_DISCOVERY outcome=stopped");
            eprintln!("code={}", stopped.code);
            eprintln!("endpoint_class={}", stopped.endpoint_class);
            match stopped.http_status {
                Some(status) => eprintln!("status_code={status}"),
                None => eprintln!("status_code=none"),
            }
            eprintln!("logout_completed={}", stopped.logout_completed);
            panic!("USGS_M2M_LIVE_DISCOVERY_STOPPED");
        }
    }
}

#[test]
#[ignore = "requires explicit local USGS credential authorization through the one-time named pipe"]
fn usgs_m2m_production_discovery_live_acceptance() {
    #[cfg(not(windows))]
    panic!("USGS_M2M_LIVE_HARNESS_WINDOWS_REQUIRED");

    #[cfg(windows)]
    match run_production_live_acceptance() {
        Ok(discovery) => {
            eprintln!("USGS_M2M_PRODUCTION_DISCOVERY outcome=completed");
            eprintln!(
                "dataset_id={}",
                discovery.descriptor().identity().dataset_id()
            );
            eprintln!(
                "dataset_alias={}",
                discovery
                    .descriptor()
                    .identity()
                    .upstream_dataset_identity()
            );
            eprintln!("candidate_count={}", discovery.candidate_count());
            eprintln!("duplicate_count={}", discovery.duplicate_count());
            eprintln!("identity_match=true");
            eprintln!("production_enabled=true");
            eprintln!("logout_completed=true");
        }
        Err(stopped) => {
            eprintln!("USGS_M2M_PRODUCTION_DISCOVERY outcome=stopped");
            print_stopped(stopped);
            panic!("USGS_M2M_PRODUCTION_DISCOVERY_STOPPED");
        }
    }
}

#[cfg(windows)]
fn run_live_acceptance() -> Result<UsgsM2mDatasetDiscoveryResponse, LiveStopped> {
    with_live_session(|session| {
        UsgsM2mDatasetDiscoveryClient::new(session)
            .discover(UsgsM2mDatasetDiscoveryRequest::landsat_c2_l2(), &|| false)
            .map_err(|error| LiveStopped::remote(error, false))
    })
}

#[cfg(windows)]
fn run_production_live_acceptance() -> Result<UsgsM2mDatasetDiscoveryResponse, LiveStopped> {
    with_live_session(|session| {
        let client = Arc::new(UsgsM2mDatasetDiscoveryClient::new(session));
        let router = UsgsM2mDatasetDiscoveryRouter::new(client);
        let request = UsgsM2mDatasetDiscoveryRouteRequest::landsat_c2_l2();
        let report = router.capability_report();
        let identity = USGS_M2M_LANDSAT_C2_L2_LIVE_IDENTITY.ok_or_else(|| {
            LiveStopped::local("USGS_M2M_LIVE_IDENTITY_ABSENT", "discovery_router", false)
        })?;

        if report.logical_dataset() != LANDSAT_C2_L2_LOGICAL_DATASET
            || report.provider_id() != LANDSAT_C2_L2_PROVIDER_ID
            || report.session_profile_id() != USGS_M2M_SESSION_PROFILE_ID
            || report.capabilities()
                != [
                    UsgsM2mCapability::DatasetDiscovery,
                    UsgsM2mCapability::RuntimeSession,
                    UsgsM2mCapability::NetworkPolicy,
                ]
            || report.state() != UsgsM2mDiscoveryState::LiveAccepted
            || report.state().as_str() != "live_accepted"
            || report.live_identity() != Some(identity)
            || report.dataset_id() != Some(identity.dataset_id())
            || report.upstream_dataset_identity() != Some(identity.dataset_alias())
            || !report.production_enabled()
            || report.network_policy() != ProviderNetworkPolicy::system()
        {
            return Err(LiveStopped::local(
                "USGS_M2M_LIVE_ROUTE_INVALID",
                "discovery_router",
                false,
            ));
        }

        let route = router
            .resolve(&request, UsgsM2mDiscoveryUse::Production)
            .map_err(|error| LiveStopped::remote(error, false))?;
        if route.route_id() != USGS_M2M_DISCOVERY_ROUTE_ID
            || route.logical_dataset() != LANDSAT_C2_L2_LOGICAL_DATASET
            || route.provider_id() != LANDSAT_C2_L2_PROVIDER_ID
            || route.session_profile_id() != USGS_M2M_SESSION_PROFILE_ID
            || route.capability() != USGS_M2M_DATASET_DISCOVERY_CAPABILITY
            || route.state() != UsgsM2mDiscoveryState::LiveAccepted
            || route.live_identity() != Some(identity)
            || route.dataset_id() != Some(identity.dataset_id())
            || route.upstream_dataset_identity() != Some(identity.dataset_alias())
            || !route.production_enabled()
        {
            return Err(LiveStopped::local(
                "USGS_M2M_LIVE_ROUTE_INVALID",
                "discovery_router",
                false,
            ));
        }

        let discovery = router
            .discover(&request, UsgsM2mDiscoveryUse::Production, &|| false)
            .map_err(|error| LiveStopped::remote(error, false))?;
        if !identity.matches_descriptor(discovery.descriptor()) {
            return Err(LiveStopped::local(
                "USGS_M2M_LIVE_IDENTITY_CONFLICT",
                "discovery_router",
                false,
            ));
        }
        Ok(discovery)
    })
}

#[cfg(windows)]
fn with_live_session<T>(
    operation: impl FnOnce(Arc<UsgsM2mSession>) -> Result<T, LiveStopped>,
) -> Result<T, LiveStopped> {
    let material = read_named_pipe_material()
        .map_err(|code| LiveStopped::local(code, "credential_pipe", false))?;
    let session = Arc::new(
        UsgsM2mSession::new(ProviderNetworkPolicy::system(), Duration::from_secs(45))
            .map_err(|error| LiveStopped::remote(error, false))?,
    );
    session
        .login(material, &|| false)
        .map_err(|error| LiveStopped::remote(error, false))?;

    let outcome = operation(Arc::clone(&session));
    let logout = session.logout(&|| false);
    let signed_out = matches!(session.status(), Ok(UsgsM2mSessionStatus::LoggedOut));
    let logout_completed = logout.is_ok() && signed_out;
    let outcome = outcome.map_err(|stopped| stopped.with_logout_completed(logout_completed))?;
    logout.map_err(|error| LiveStopped::remote(error, false))?;
    if !signed_out {
        return Err(LiveStopped::local(
            "USGS_M2M_LIVE_SESSION_STATUS_INVALID",
            "runtime_session",
            false,
        ));
    }
    Ok(outcome)
}

#[cfg(windows)]
#[derive(Clone, Copy)]
struct LiveStopped {
    code: &'static str,
    endpoint_class: &'static str,
    http_status: Option<u16>,
    logout_completed: bool,
}

#[cfg(windows)]
impl LiveStopped {
    const fn local(
        code: &'static str,
        endpoint_class: &'static str,
        logout_completed: bool,
    ) -> Self {
        Self {
            code,
            endpoint_class,
            http_status: None,
            logout_completed,
        }
    }

    fn remote(error: UsgsM2mError, logout_completed: bool) -> Self {
        Self {
            code: error.code_str(),
            endpoint_class: endpoint_class(error.endpoint()),
            http_status: error.http_status(),
            logout_completed,
        }
    }

    const fn with_logout_completed(mut self, logout_completed: bool) -> Self {
        self.logout_completed = logout_completed;
        self
    }
}

#[cfg(windows)]
fn print_stopped(stopped: LiveStopped) {
    eprintln!("code={}", stopped.code);
    eprintln!("endpoint_class={}", stopped.endpoint_class);
    match stopped.http_status {
        Some(status) => eprintln!("status_code={status}"),
        None => eprintln!("status_code=none"),
    }
    eprintln!("logout_completed={}", stopped.logout_completed);
}

#[cfg(windows)]
const fn endpoint_class(endpoint: UsgsM2mEndpointClass) -> &'static str {
    match endpoint {
        UsgsM2mEndpointClass::LoginToken => "login_token",
        UsgsM2mEndpointClass::Permissions => "permissions",
        UsgsM2mEndpointClass::DatasetDiscovery => "dataset_discovery",
        UsgsM2mEndpointClass::DatasetFilters => "dataset_filters",
        UsgsM2mEndpointClass::SceneSearch => "scene_search",
        UsgsM2mEndpointClass::DownloadOptions => "download_options",
        UsgsM2mEndpointClass::DownloadRequest => "download_request",
        UsgsM2mEndpointClass::DownloadRetrieve => "download_retrieve",
        UsgsM2mEndpointClass::BrowseFetch => "browse_fetch",
        UsgsM2mEndpointClass::Logout => "logout",
        UsgsM2mEndpointClass::RuntimeSession => "runtime_session",
        UsgsM2mEndpointClass::DiscoveryRouter => "discovery_router",
        UsgsM2mEndpointClass::SceneSearchRouter => "scene_search_router",
    }
}

#[cfg(windows)]
fn read_named_pipe_material() -> Result<UsgsM2mLoginMaterial, &'static str> {
    let mut pipe = OpenOptions::new()
        .read(true)
        .open(LIVE_CREDENTIAL_PIPE)
        .map_err(|_| "USGS_M2M_LIVE_PIPE_UNAVAILABLE")?;
    read_login_material(&mut pipe)
}

fn read_login_material(pipe: &mut impl Read) -> Result<UsgsM2mLoginMaterial, &'static str> {
    let mut username = read_bounded_utf8(pipe, MAX_USERNAME_BYTES)?;
    let mut token = read_bounded_utf8(pipe, MAX_APPLICATION_TOKEN_BYTES)?;
    let mut trailing = Zeroizing::new([0_u8; 1]);
    if pipe
        .read(&mut *trailing)
        .map_err(|_| "USGS_M2M_LIVE_PIPE_INVALID")?
        != 0
    {
        return Err("USGS_M2M_LIVE_PIPE_INVALID");
    }

    let token = UsgsM2mApplicationToken::new(std::mem::take(&mut *token))
        .map_err(|_| "USGS_M2M_LIVE_MATERIAL_INVALID")?;
    UsgsM2mLoginMaterial::new(std::mem::take(&mut *username), token)
        .map_err(|_| "USGS_M2M_LIVE_MATERIAL_INVALID")
}

fn read_bounded_utf8(
    pipe: &mut impl Read,
    maximum: usize,
) -> Result<Zeroizing<String>, &'static str> {
    let mut length = Zeroizing::new([0_u8; 4]);
    pipe.read_exact(&mut *length)
        .map_err(|_| "USGS_M2M_LIVE_PIPE_INVALID")?;
    let length = u32::from_le_bytes(*length) as usize;
    if length == 0 || length > maximum {
        return Err("USGS_M2M_LIVE_PIPE_INVALID");
    }
    let mut bytes = Zeroizing::new(vec![0_u8; length]);
    pipe.read_exact(&mut bytes)
        .map_err(|_| "USGS_M2M_LIVE_PIPE_INVALID")?;
    std::str::from_utf8(&bytes)
        .map(str::to_owned)
        .map(Zeroizing::new)
        .map_err(|_| "USGS_M2M_LIVE_PIPE_INVALID")
}

fn framed_login(username: &[u8], token: &[u8]) -> Vec<u8> {
    let mut payload = Vec::new();
    payload.extend_from_slice(&(username.len() as u32).to_le_bytes());
    payload.extend_from_slice(username);
    payload.extend_from_slice(&(token.len() as u32).to_le_bytes());
    payload.extend_from_slice(token);
    payload
}

#[test]
fn named_pipe_payload_accepts_one_bounded_utf8_login_without_debug_secrets() {
    let username = "synthetic-live-user";
    let token = "synthetic-live-application-token";
    let material = read_login_material(&mut Cursor::new(framed_login(
        username.as_bytes(),
        token.as_bytes(),
    )))
    .unwrap();
    let debug = format!("{material:?}");
    assert!(!debug.contains(username));
    assert!(!debug.contains(token));
}

#[test]
fn named_pipe_payload_rejects_empty_and_oversized_fields_before_allocation() {
    for payload in [
        framed_login(b"", b"synthetic-token"),
        {
            let mut payload = Vec::new();
            payload.extend_from_slice(&((MAX_USERNAME_BYTES + 1) as u32).to_le_bytes());
            payload
        },
        framed_login(b"synthetic-user", b""),
        {
            let mut payload = framed_login(b"synthetic-user", b"synthetic-token");
            payload.truncate(4 + b"synthetic-user".len());
            payload.extend_from_slice(&((MAX_APPLICATION_TOKEN_BYTES + 1) as u32).to_le_bytes());
            payload
        },
    ] {
        assert_eq!(
            read_login_material(&mut Cursor::new(payload)).unwrap_err(),
            "USGS_M2M_LIVE_PIPE_INVALID"
        );
    }
}

#[test]
fn named_pipe_payload_rejects_non_utf8_truncation_and_trailing_data() {
    let mut non_utf8 = framed_login(&[0xff, 0xfe], b"synthetic-token");
    let mut truncated = framed_login(b"synthetic-user", b"synthetic-token");
    truncated.pop();
    let mut trailing = framed_login(b"synthetic-user", b"synthetic-token");
    trailing.push(0);

    for payload in [&mut non_utf8, &mut truncated, &mut trailing] {
        assert_eq!(
            read_login_material(&mut Cursor::new(payload)).unwrap_err(),
            "USGS_M2M_LIVE_PIPE_INVALID"
        );
    }
}

#[test]
fn named_pipe_payload_rejects_a_second_framed_login() {
    let first = framed_login(b"synthetic-user-one", b"synthetic-token-one");
    let second = framed_login(b"synthetic-user-two", b"synthetic-token-two");
    let mut payload = first;
    payload.extend_from_slice(&second);
    assert_eq!(
        read_login_material(&mut Cursor::new(payload)).unwrap_err(),
        "USGS_M2M_LIVE_PIPE_INVALID"
    );
}
