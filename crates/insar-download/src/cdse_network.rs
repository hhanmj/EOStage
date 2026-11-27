use std::error::Error;
use std::sync::Arc;
use std::time::Duration;

use insar_core::{ProviderNetworkPolicy, ProviderResult, ProxyPolicy};
use insar_provider::cdse::catalog_client::CdseODataCatalogClient;
use reqwest::blocking::Client;
use reqwest::redirect::Policy;
use reqwest::Url;

use crate::asset_resolver::{Clock, CredentialProvider, ResolveContext, SystemClock};
use crate::cdse_oauth::{CdseAuthError, CdseOAuthSession, ReqwestCdseTokenEndpoint};
use crate::{HttpTransfer, TransferOptions, TransferRequestError, CDSE_IDENTITY_TOKEN_ENDPOINT};

/// Redacted transport stage for an explicit credential-free CDSE diagnostic.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CdseNetworkDiagnosticStage {
    DnsUnavailable,
    TcpUnavailable,
    TlsHandshakeFailed,
    ProxyConfigurationRejected,
    Timeout,
    HttpReached,
    TransportUnavailable,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CdseNetworkDiagnostic {
    pub route: ProxyPolicy,
    pub stage: CdseNetworkDiagnosticStage,
}

/// Testable, credential-free transport port. It receives only a bounded route
/// policy and can never receive username, password, TOTP, or Bearer material.
pub trait CdseNetworkProbe: Send + Sync {
    fn probe(&self, network_policy: ProviderNetworkPolicy) -> CdseNetworkDiagnosticStage;
}

/// Fixed-origin CDSE reachability probe. `diagnose` sends one unauthenticated
/// `HEAD` request, follows no redirect, and considers any HTTP status reachable.
pub struct ReqwestCdseNetworkProbe {
    network_policy: ProviderNetworkPolicy,
    client: Client,
    endpoint: Url,
}

impl ReqwestCdseNetworkProbe {
    pub fn new(network_policy: ProviderNetworkPolicy) -> Result<Self, CdseNetworkDiagnosticStage> {
        let endpoint = Url::parse(CDSE_IDENTITY_TOKEN_ENDPOINT)
            .map_err(|_| CdseNetworkDiagnosticStage::TransportUnavailable)?;
        if endpoint.scheme() != "https"
            || endpoint.host_str() != Some("identity.dataspace.copernicus.eu")
            || endpoint.port_or_known_default() != Some(443)
            || endpoint.path() != "/auth/realms/CDSE/protocol/openid-connect/token"
            || !endpoint.username().is_empty()
            || endpoint.password().is_some()
            || endpoint.query().is_some()
            || endpoint.fragment().is_some()
        {
            return Err(CdseNetworkDiagnosticStage::TransportUnavailable);
        }
        let client = Client::builder()
            .redirect(Policy::none())
            .retry(reqwest::retry::never())
            .cookie_store(false)
            .referer(false)
            .connect_timeout(std::time::Duration::from_secs(15))
            .timeout(std::time::Duration::from_secs(45));
        let client = insar_provider::network_proxy::blocking_builder_with_proxy(
            client,
            network_policy.route(),
        )
        .build()
        .map_err(|_| CdseNetworkDiagnosticStage::ProxyConfigurationRejected)?;
        Ok(Self {
            network_policy,
            client,
            endpoint,
        })
    }

    pub const fn network_policy(&self) -> ProviderNetworkPolicy {
        self.network_policy
    }
}

impl CdseNetworkProbe for ReqwestCdseNetworkProbe {
    fn probe(&self, network_policy: ProviderNetworkPolicy) -> CdseNetworkDiagnosticStage {
        if network_policy != self.network_policy {
            return CdseNetworkDiagnosticStage::ProxyConfigurationRejected;
        }
        match self.client.head(self.endpoint.clone()).send() {
            Ok(_response) => CdseNetworkDiagnosticStage::HttpReached,
            Err(error) => classify_request_error(&error),
        }
    }
}

pub struct CdseNetworkDiagnostics {
    network_policy: ProviderNetworkPolicy,
    probe: Arc<dyn CdseNetworkProbe>,
}

impl CdseNetworkDiagnostics {
    pub fn new(network_policy: ProviderNetworkPolicy, probe: Arc<dyn CdseNetworkProbe>) -> Self {
        Self {
            network_policy,
            probe,
        }
    }

    pub const fn network_policy(&self) -> ProviderNetworkPolicy {
        self.network_policy
    }

    pub fn diagnose(&self) -> CdseNetworkDiagnostic {
        CdseNetworkDiagnostic {
            route: self.network_policy.route(),
            stage: self.probe.probe(self.network_policy),
        }
    }
}

/// Runtime-only CDSE construction boundary. It retains only a bounded route
/// policy and deliberately does not register a catalogue client, Resolver, or
/// credential source with any production runtime.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CdseNetworkFoundation {
    network_policy: ProviderNetworkPolicy,
}

impl Default for CdseNetworkFoundation {
    fn default() -> Self {
        Self::new(ProviderNetworkPolicy::system())
    }
}

impl CdseNetworkFoundation {
    pub const fn new(network_policy: ProviderNetworkPolicy) -> Self {
        Self { network_policy }
    }

    pub const fn network_policy(&self) -> ProviderNetworkPolicy {
        self.network_policy
    }

    /// Constructs the exact CDSE Identity endpoint. Construction is offline;
    /// a token request is only made when a caller explicitly logs in later.
    pub fn token_endpoint(&self) -> Result<ReqwestCdseTokenEndpoint, CdseAuthError> {
        ReqwestCdseTokenEndpoint::new(self.network_policy)
    }

    /// Constructs an empty, runtime-only OAuth session using the exact Identity
    /// endpoint. It neither reads credentials nor performs a network request.
    pub fn oauth_session(&self) -> Result<CdseOAuthSession, CdseAuthError> {
        CdseOAuthSession::new(
            self.network_policy,
            Arc::new(self.token_endpoint()?),
            Arc::new(SystemClock),
        )
    }

    /// Constructs the reviewed, default-unregistered OData client with this
    /// exact route policy. The caller still owns any later search invocation.
    pub fn catalogue_client(&self, timeout: Duration) -> ProviderResult<CdseODataCatalogClient> {
        CdseODataCatalogClient::with_network_policy(timeout, self.network_policy)
    }

    pub const fn archive_runtime(&self) -> CdseArchiveNetworkRuntime {
        CdseArchiveNetworkRuntime::new(self.network_policy)
    }
}

/// Explicit archive construction owner. It does not register a Resolver or
/// start a transfer; it only carries one route choice into existing APIs.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CdseArchiveNetworkRuntime {
    network_policy: ProviderNetworkPolicy,
}

impl CdseArchiveNetworkRuntime {
    pub const fn new(network_policy: ProviderNetworkPolicy) -> Self {
        Self { network_policy }
    }

    pub const fn network_policy(&self) -> ProviderNetworkPolicy {
        self.network_policy
    }

    pub fn resolve_context<'a>(
        &self,
        credentials: &'a dyn CredentialProvider,
        cancelled: &'a (dyn Fn() -> bool + Sync),
        clock: &'a dyn Clock,
    ) -> ResolveContext<'a> {
        ResolveContext::new(credentials, cancelled, clock)
            .with_proxy_policy(self.network_policy.route())
    }

    pub fn transfer(&self, options: TransferOptions) -> Result<HttpTransfer, TransferRequestError> {
        HttpTransfer::new_with_proxy_policy(options, self.network_policy.route())
    }
}

fn classify_request_error(error: &reqwest::Error) -> CdseNetworkDiagnosticStage {
    if error.is_timeout() {
        return CdseNetworkDiagnosticStage::Timeout;
    }
    let mut source = error.source();
    while let Some(current) = source {
        if let Some(io_error) = current.downcast_ref::<std::io::Error>() {
            return match io_error.kind() {
                std::io::ErrorKind::NotFound | std::io::ErrorKind::AddrNotAvailable => {
                    CdseNetworkDiagnosticStage::DnsUnavailable
                }
                std::io::ErrorKind::ConnectionRefused
                | std::io::ErrorKind::ConnectionAborted
                | std::io::ErrorKind::ConnectionReset
                | std::io::ErrorKind::NotConnected => CdseNetworkDiagnosticStage::TcpUnavailable,
                std::io::ErrorKind::InvalidData => CdseNetworkDiagnosticStage::TlsHandshakeFailed,
                std::io::ErrorKind::TimedOut => CdseNetworkDiagnosticStage::Timeout,
                _ => CdseNetworkDiagnosticStage::TransportUnavailable,
            };
        }
        source = current.source();
    }
    if error.is_connect() {
        CdseNetworkDiagnosticStage::TcpUnavailable
    } else {
        CdseNetworkDiagnosticStage::TransportUnavailable
    }
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicUsize, Ordering};

    use super::*;
    use crate::asset_resolver::{CredentialMaterial, CredentialProviderError, SystemClock};

    struct PanicCredentials;

    impl CredentialProvider for PanicCredentials {
        fn resolve(&self, _profile: &str) -> Result<CredentialMaterial, CredentialProviderError> {
            panic!("credential-free diagnostic must not resolve credentials")
        }
    }

    struct FixtureProbe {
        expected: ProviderNetworkPolicy,
        stage: CdseNetworkDiagnosticStage,
        calls: AtomicUsize,
    }

    impl CdseNetworkProbe for FixtureProbe {
        fn probe(&self, policy: ProviderNetworkPolicy) -> CdseNetworkDiagnosticStage {
            assert_eq!(policy, self.expected);
            self.calls.fetch_add(1, Ordering::SeqCst);
            self.stage
        }
    }

    #[test]
    fn system_is_the_default_and_direct_reaches_only_existing_route_seams() {
        let credentials = PanicCredentials;
        let cancelled = || false;
        let clock = SystemClock;
        let system = CdseArchiveNetworkRuntime::new(ProviderNetworkPolicy::default());
        assert_eq!(system.network_policy(), ProviderNetworkPolicy::system());
        assert_eq!(
            system
                .resolve_context(&credentials, &cancelled, &clock)
                .proxy_policy(),
            ProxyPolicy::System
        );
        assert_eq!(
            system
                .transfer(TransferOptions::default())
                .expect("system transfer")
                .proxy_policy(),
            ProxyPolicy::System
        );

        let direct = CdseArchiveNetworkRuntime::new(ProviderNetworkPolicy::direct());
        assert_eq!(
            direct
                .resolve_context(&credentials, &cancelled, &clock)
                .proxy_policy(),
            ProxyPolicy::Direct
        );
        assert_eq!(
            direct
                .transfer(TransferOptions::default())
                .expect("direct transfer")
                .proxy_policy(),
            ProxyPolicy::Direct
        );
    }

    #[test]
    fn foundation_applies_one_explicit_route_to_identity_session_catalogue_and_archive() {
        assert_eq!(
            CdseNetworkFoundation::default().network_policy(),
            ProviderNetworkPolicy::system()
        );

        for policy in [
            ProviderNetworkPolicy::system(),
            ProviderNetworkPolicy::direct(),
        ] {
            let foundation = CdseNetworkFoundation::new(policy);

            assert_eq!(foundation.network_policy(), policy);
            assert_eq!(
                foundation
                    .token_endpoint()
                    .expect("exact identity endpoint")
                    .network_policy(),
                policy
            );
            assert_eq!(
                foundation
                    .oauth_session()
                    .expect("empty runtime OAuth session")
                    .network_policy(),
                policy
            );
            assert_eq!(
                foundation
                    .catalogue_client(Duration::from_secs(1))
                    .expect("default-unregistered catalogue client")
                    .network_policy(),
                policy
            );
            assert_eq!(foundation.archive_runtime().network_policy(), policy);
        }
    }

    #[test]
    fn diagnostics_require_separate_explicit_system_and_direct_calls() {
        let system_policy = ProviderNetworkPolicy::system();
        let system_probe = Arc::new(FixtureProbe {
            expected: system_policy,
            stage: CdseNetworkDiagnosticStage::HttpReached,
            calls: AtomicUsize::new(0),
        });
        let system = CdseNetworkDiagnostics::new(
            system_policy,
            Arc::clone(&system_probe) as Arc<dyn CdseNetworkProbe>,
        );
        let direct_policy = ProviderNetworkPolicy::direct();
        let direct_probe = Arc::new(FixtureProbe {
            expected: direct_policy,
            stage: CdseNetworkDiagnosticStage::TlsHandshakeFailed,
            calls: AtomicUsize::new(0),
        });
        let direct = CdseNetworkDiagnostics::new(
            direct_policy,
            Arc::clone(&direct_probe) as Arc<dyn CdseNetworkProbe>,
        );

        assert_eq!(
            system.diagnose(),
            CdseNetworkDiagnostic {
                route: ProxyPolicy::System,
                stage: CdseNetworkDiagnosticStage::HttpReached,
            }
        );
        assert_eq!(
            direct.diagnose(),
            CdseNetworkDiagnostic {
                route: ProxyPolicy::Direct,
                stage: CdseNetworkDiagnosticStage::TlsHandshakeFailed,
            }
        );
        assert_eq!(system_probe.calls.load(Ordering::SeqCst), 1);
        assert_eq!(direct_probe.calls.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn diagnostics_only_expose_redacted_route_and_stage() {
        let policy = ProviderNetworkPolicy::direct();
        let probe = Arc::new(FixtureProbe {
            expected: policy,
            stage: CdseNetworkDiagnosticStage::TlsHandshakeFailed,
            calls: AtomicUsize::new(0),
        });
        let result = CdseNetworkDiagnostics::new(policy, probe).diagnose();

        assert_eq!(
            format!("{result:?}"),
            "CdseNetworkDiagnostic { route: Direct, stage: TlsHandshakeFailed }"
        );
    }

    #[test]
    fn reqwest_probe_construction_keeps_system_and_direct_distinct_without_requesting_network() {
        assert_eq!(
            ReqwestCdseNetworkProbe::new(ProviderNetworkPolicy::system())
                .expect("system probe")
                .network_policy(),
            ProviderNetworkPolicy::system()
        );
        assert_eq!(
            ReqwestCdseNetworkProbe::new(ProviderNetworkPolicy::direct())
                .expect("direct probe")
                .network_policy(),
            ProviderNetworkPolicy::direct()
        );
    }

    #[test]
    fn reqwest_probe_rejects_route_changes_without_attempting_a_request() {
        let probe =
            ReqwestCdseNetworkProbe::new(ProviderNetworkPolicy::system()).expect("system probe");

        assert_eq!(
            probe.probe(ProviderNetworkPolicy::direct()),
            CdseNetworkDiagnosticStage::ProxyConfigurationRejected
        );
    }

    #[test]
    #[ignore = "requires explicit maintainer authorization for a credential-free public CDSE diagnostic"]
    fn optional_live_diagnostic_is_opt_in_and_never_reads_credentials() {
        if std::env::var("INSAR_CDSE_NETWORK_DIAGNOSTIC")
            .ok()
            .as_deref()
            != Some("1")
        {
            return;
        }
        let probe = Arc::new(
            ReqwestCdseNetworkProbe::new(ProviderNetworkPolicy::system()).expect("system probe"),
        );
        let result = CdseNetworkDiagnostics::new(ProviderNetworkPolicy::system(), probe).diagnose();
        assert_eq!(result.route, ProxyPolicy::System);
    }
}
