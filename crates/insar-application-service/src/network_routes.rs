use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Mutex;
use std::time::{Duration, Instant};

use insar_core::{ProviderNetworkPolicy, TaskLifecycleStatus, TaskRuntimeSnapshot};

/// While transfers run, a pending route change re-reads the Store at most
/// this often instead of on every status poll.
const BUSY_RECHECK: Duration = Duration::from_secs(3);

/// A saved route change waiting for an idle runtime. Requests are counted so
/// a save that races an apply is never marked applied by the older apply.
#[derive(Default)]
pub(crate) struct PendingNetworkApply {
    requested: AtomicU64,
    applied: AtomicU64,
    next_check: Mutex<Option<Instant>>,
}

#[derive(Debug, PartialEq, Eq)]
enum ApplyOutcome {
    Applied,
    /// The Store or a catalogue request showed transfers in progress.
    Busy,
    /// A lock was held by another operation; nothing was inspected.
    Contended,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ServiceRoute {
    Ordinary,
    Bhoonidhi,
}

pub(crate) fn default_policy(service: ServiceRoute) -> ProviderNetworkPolicy {
    match service {
        ServiceRoute::Ordinary => ProviderNetworkPolicy::direct(),
        ServiceRoute::Bhoonidhi => ProviderNetworkPolicy::system(),
    }
}

/// The route a service uses under the saved network settings. Ordinary data
/// sources follow the setting (direct by default). Bhoonidhi keeps the system
/// route unless the user configured a manual proxy, which it then also uses.
pub(crate) fn policy_for_settings(
    service: ServiceRoute,
    settings: &crate::ApplicationNetworkSettings,
) -> ProviderNetworkPolicy {
    match settings.proxy_mode() {
        crate::NetworkProxyMode::Manual => manual_policy(&settings.proxy_url)
            // Saved settings are validated on read and save; an unparsable
            // authority keeps the service default instead of a guessed route.
            .unwrap_or_else(|| default_policy(service)),
        crate::NetworkProxyMode::Auto => ProviderNetworkPolicy::system(),
        crate::NetworkProxyMode::Direct => default_policy(service),
    }
}

fn manual_policy(proxy_url: &str) -> Option<ProviderNetworkPolicy> {
    let url = reqwest::Url::parse(proxy_url.trim()).ok()?;
    let proxy = insar_core::ManualProxy::new(url.host_str()?, url.port_or_known_default()?).ok()?;
    Some(ProviderNetworkPolicy::manual(proxy))
}

impl crate::ApplicationService {
    /// The route for a client constructed now, from the saved settings.
    pub(crate) fn network_policy(&self, service: ServiceRoute) -> ProviderNetworkPolicy {
        let settings = match self.get_network_settings() {
            Ok(settings) => settings,
            Err(crate::ApplicationServiceError::StorePathRequired) => {
                crate::ApplicationNetworkSettings::default()
            }
            Err(err) => {
                eprintln!(
                    "[network_routes] network_policy fallback to default: code={}",
                    err.code()
                );
                crate::ApplicationNetworkSettings::default()
            }
        };
        policy_for_settings(service, &settings)
    }

    pub(crate) fn request_network_policy_apply(&self) {
        let state = &self.network_apply;
        state.requested.fetch_add(1, Ordering::AcqRel);
        *state
            .next_check
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = None;
        self.apply_pending_network_policy();
    }

    /// Cheap on the hot path: only a pending settings change does any work,
    /// and while transfers run the Store is re-read at most every few seconds.
    pub(crate) fn apply_pending_network_policy(&self) {
        let state = &self.network_apply;
        let target = state.requested.load(Ordering::Acquire);
        if state.applied.load(Ordering::Acquire) >= target {
            return;
        }
        // Another caller holding it is applying right now.
        let mut next_check = match state.next_check.try_lock() {
            Ok(guard) => guard,
            Err(std::sync::TryLockError::Poisoned(poisoned)) => poisoned.into_inner(),
            Err(std::sync::TryLockError::WouldBlock) => return,
        };
        if next_check.is_some_and(|at| Instant::now() < at) {
            return;
        }
        match self.apply_network_policy_if_idle() {
            ApplyOutcome::Applied => {
                state.applied.fetch_max(target, Ordering::AcqRel);
                *next_check = None;
            }
            ApplyOutcome::Busy => *next_check = Some(Instant::now() + BUSY_RECHECK),
            ApplyOutcome::Contended => {}
        }
    }

    /// Retire process sessions and the business runtime built on a route that
    /// no longer matches the saved settings, so new requests are built on the
    /// new route. Never runs while an item transfers or a catalogue request is
    /// active: running tasks keep their connection and the change stays
    /// pending. Every lock is tried, never awaited, so callers cannot deadlock.
    /// Holding the runtime lock keeps task creation and resume out while the
    /// Store shows no task the scheduler could start.
    fn apply_network_policy_if_idle(&self) -> ApplyOutcome {
        let settings = match self.get_network_settings() {
            Ok(settings) => settings,
            Err(crate::ApplicationServiceError::StorePathRequired) => {
                crate::ApplicationNetworkSettings::default()
            }
            Err(err) => {
                eprintln!(
                    "[network_routes] deferring network policy apply: code={}",
                    err.code()
                );
                return ApplyOutcome::Busy;
            }
        };
        let ordinary = policy_for_settings(ServiceRoute::Ordinary, &settings);
        let bhoonidhi = policy_for_settings(ServiceRoute::Bhoonidhi, &settings);
        let Ok(_gate) = self.catalogue_policy_gate.try_lock() else {
            return ApplyOutcome::Contended;
        };
        let Ok(mut runtime) = self.runtime.try_lock() else {
            return ApplyOutcome::Contended;
        };
        let Ok(mut cdse) = self.cdse_session.try_lock() else {
            return ApplyOutcome::Contended;
        };
        let Ok(mut usgs) = self.usgs_session.try_lock() else {
            return ApplyOutcome::Contended;
        };
        let Ok(mut bhoonidhi_session) = self.bhoonidhi_session.try_lock() else {
            return ApplyOutcome::Contended;
        };
        let cdse_stale = cdse
            .as_ref()
            .is_some_and(|session| session.network_policy != ordinary);
        let usgs_stale = usgs
            .as_ref()
            .is_some_and(|session| session.session.network_policy() != ordinary);
        let bhoonidhi_stale = bhoonidhi_session
            .as_ref()
            .is_some_and(|session| session.session.network_policy() != bhoonidhi);
        // A runtime is built from the CDSE and Bhoonidhi sessions; without them
        // it was built before any change and still holds the startup route.
        let runtime_stale =
            runtime.is_some() && (cdse_stale || usgs_stale || bhoonidhi_stale || cdse.is_none());
        if !(cdse_stale || usgs_stale || bhoonidhi_stale || runtime_stale) {
            return ApplyOutcome::Applied;
        }
        if let Some(existing) = runtime.as_ref() {
            if !matches!(crate::runtime_has_active_cdse_work(existing), Ok(false)) {
                return ApplyOutcome::Busy;
            }
        }
        let previous_cdse = if cdse_stale { cdse.take() } else { None };
        let previous_usgs = if usgs_stale { usgs.take() } else { None };
        let previous_bhoonidhi = if bhoonidhi_stale {
            bhoonidhi_session.take()
        } else {
            None
        };
        let previous_runtime = if runtime_stale { runtime.take() } else { None };
        drop((bhoonidhi_session, usgs, cdse, runtime, _gate));
        if let Some(previous) = previous_cdse {
            let _ = previous.session.logout();
        }
        drop((previous_usgs, previous_bhoonidhi, previous_runtime));
        ApplyOutcome::Applied
    }
}

/// Whether the scheduler has, or may start, a transfer for these tasks. A
/// task between items has no active item, yet the scheduler can start its
/// next queued item right after the check, and dropping the old runtime would
/// cancel that worker.
pub(crate) fn runtimes_may_transfer(runtimes: &[TaskRuntimeSnapshot]) -> bool {
    runtimes.iter().any(|snapshot| {
        !snapshot.active_item_ids.is_empty()
            || (!snapshot.task_paused
                && matches!(
                    snapshot.status,
                    TaskLifecycleStatus::Queued | TaskLifecycleStatus::Running
                ))
    })
}

/// One credential-free provider reachability probe target. The site URL is
/// only used by the UI as the provider's homepage link; the probe itself
/// always hits `endpoint`.
pub(crate) struct ProviderProbeTarget {
    pub id: &'static str,
    pub endpoint: std::borrow::Cow<'static, str>,
    pub site: &'static str,
}

/// Fixed, credential-free entry endpoints of every data provider the
/// connectivity card lists. Constants come from the provider crates so the
/// probe follows the same origins the real clients use.
pub(crate) fn provider_probe_targets() -> Vec<ProviderProbeTarget> {
    vec![
        ProviderProbeTarget {
            id: "cdse",
            endpoint: insar_download::CDSE_IDENTITY_TOKEN_ENDPOINT.into(),
            site: "https://dataspace.copernicus.eu/",
        },
        ProviderProbeTarget {
            id: "earthdata",
            endpoint: "https://api.daac.asf.alaska.edu/services/search/param".into(),
            site: "https://search.asf.alaska.edu/",
        },
        ProviderProbeTarget {
            id: "bhoonidhi",
            endpoint: insar_download::BHOONIDHI_TOKEN_ENDPOINT.into(),
            site: "https://bhoonidhi.nrsc.gov.in/",
        },
        ProviderProbeTarget {
            id: "usgs_m2m",
            endpoint: insar_provider::usgs_m2m::USGS_M2M_STABLE_ORIGIN.into(),
            site: "https://earthexplorer.usgs.gov/",
        },
        ProviderProbeTarget {
            id: "mpc",
            endpoint: insar_provider::catalog::MPC_REVIEWED_STAC_ENDPOINT.into(),
            site: "https://planetarycomputer.microsoft.com/",
        },
        ProviderProbeTarget {
            id: "opentopography",
            endpoint: "https://portal.opentopography.org/API/globaldem".into(),
            site: "https://portal.opentopography.org/",
        },
        ProviderProbeTarget {
            id: "gee",
            endpoint: "https://earthengine.googleapis.com/$discovery/rest?version=v1".into(),
            site: "https://earthengine.google.com/",
        },
        ProviderProbeTarget {
            id: "gacos",
            endpoint: insar_provider::gacos_delivery::GACOS_WEB_ROOT.into(),
            site: insar_provider::gacos_delivery::GACOS_WEB_ROOT,
        },
        ProviderProbeTarget {
            id: "comet_licsar",
            endpoint: "https://gws-access.jasmin.ac.uk/public/nceo_geohazards/LiCSAR_products/"
                .into(),
            site: "https://comet.nerc.ac.uk/COMET-LiCS-portal/",
        },
    ]
}

/// The provider homepage the settings card links to, by provider id.
pub(crate) fn provider_probe_site(id: &str) -> Option<&'static str> {
    provider_probe_targets()
        .into_iter()
        .find(|target| target.id == id)
        .map(|target| target.site)
}

/// Per-endpoint outcome of one bounded connectivity probe run.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct ProviderProbeOutcome {
    pub id: &'static str,
    pub ok: bool,
    pub latency_ms: u64,
    pub http_status: Option<u16>,
    /// One of a fixed stage set mirroring the single-endpoint probe:
    /// http_reached / timeout / transport_unavailable /
    /// proxy_configuration_rejected.
    pub error_code: &'static str,
}

/// Probe every provider entry endpoint concurrently with the saved proxy
/// policy, one credential-free HEAD request each, and report latency. Every
/// request finishes within the bounded timeout even when the endpoint hangs;
/// errors never abort the other endpoints. Results keep the fixed catalogue
/// order regardless of completion order.
pub(crate) fn probe_providers_for_settings(
    settings: &crate::ApplicationNetworkSettings,
) -> Vec<ProviderProbeOutcome> {
    let targets = provider_probe_targets();
    probe_targets_with_settings(&targets, settings)
}

/// The probe route a provider id uses. It must match the route the provider's
/// real client uses so the probe reports the connectivity the user actually
/// gets: Bhoonidhi's client inherits the system proxy even in Direct mode,
/// while ordinary providers go direct.
pub(crate) fn probe_route_for(id: &str) -> ServiceRoute {
    match id {
        "bhoonidhi" => ServiceRoute::Bhoonidhi,
        _ => ServiceRoute::Ordinary,
    }
}

/// The probe proxy policy for a provider id under the saved settings, built by
/// the same policy function the real clients use.
pub(crate) fn probe_policy_for(
    id: &str,
    settings: &crate::ApplicationNetworkSettings,
) -> insar_core::ProxyPolicy {
    policy_for_settings(probe_route_for(id), settings).route()
}

pub(crate) fn probe_targets_with_settings(
    targets: &[ProviderProbeTarget],
    settings: &crate::ApplicationNetworkSettings,
) -> Vec<ProviderProbeOutcome> {
    use reqwest::blocking::Client;
    use reqwest::redirect::Policy;
    use std::sync::mpsc;
    use std::time::Duration;

    let rejected = |targets: &[ProviderProbeTarget]| {
        targets
            .iter()
            .map(|target| ProviderProbeOutcome {
                id: target.id,
                ok: false,
                latency_ms: 0,
                http_status: None,
                error_code: "proxy_configuration_rejected",
            })
            .collect::<Vec<_>>()
    };
    // A saved manual proxy is validated on save; settings that reach the probe
    // with an authority reqwest cannot use reject every endpoint up front,
    // exactly like the single CDSE probe.
    if settings.proxy_mode() == crate::NetworkProxyMode::Manual
        && reqwest::Proxy::all(settings.proxy_url.trim()).is_err()
    {
        return rejected(targets);
    }
    // One client per provider route policy, so each probe follows exactly the
    // proxy route that provider's real client uses.
    let mut clients = Vec::with_capacity(targets.len());
    for target in targets {
        let builder = Client::builder()
            .redirect(Policy::none())
            .retry(reqwest::retry::never())
            .cookie_store(false)
            .referer(false)
            .connect_timeout(Duration::from_secs(5))
            .timeout(Duration::from_secs(8));
        let builder = match probe_policy_for(target.id, settings) {
            insar_core::ProxyPolicy::System => builder,
            insar_core::ProxyPolicy::Direct => builder.no_proxy(),
            insar_core::ProxyPolicy::Manual(manual) => match reqwest::Proxy::all(manual.url()) {
                Ok(proxy) => builder.no_proxy().proxy(proxy),
                Err(_) => return rejected(targets),
            },
        };
        match builder.build() {
            Ok(client) => clients.push(client),
            Err(_) => return rejected(targets),
        }
    }

    // The per-request timeout bounds a hanging endpoint, but not a worker that
    // panics before sending: an overall deadline turns a missing outcome into
    // an explicit failed entry instead of an omitted provider.
    const PROBE_DEADLINE: Duration = Duration::from_secs(15);
    let deadline = Instant::now() + PROBE_DEADLINE;
    let mut received: std::collections::HashMap<&'static str, ProviderProbeOutcome> =
        std::collections::HashMap::new();
    let mut waiting: std::collections::HashSet<&'static str> =
        targets.iter().map(|target| target.id).collect();
    let (sender, receiver) = mpsc::channel::<ProviderProbeOutcome>();
    for (target, client) in targets.iter().zip(&clients) {
        let client = client.clone();
        let sender = sender.clone();
        let endpoint = target.endpoint.clone();
        let id = target.id;
        let spawn_result = std::thread::Builder::new()
            .name(format!("net-probe-{id}"))
            .spawn(move || {
                let _ = sender.send(probe_endpoint_with_client(&client, id, &endpoint));
            });
        if spawn_result.is_err() {
            waiting.remove(id);
            received.insert(
                id,
                ProviderProbeOutcome {
                    id,
                    ok: false,
                    latency_ms: 0,
                    http_status: None,
                    error_code: "probe_spawn_failed",
                },
            );
        }
    }
    drop(sender);
    while !waiting.is_empty() {
        let Some(budget) = deadline.checked_duration_since(Instant::now()) else {
            break;
        };
        match receiver.recv_timeout(budget) {
            Ok(outcome) => {
                if waiting.remove(outcome.id) {
                    received.insert(outcome.id, outcome);
                }
            }
            Err(mpsc::RecvTimeoutError::Timeout) => break,
            Err(mpsc::RecvTimeoutError::Disconnected) => break,
        }
    }
    targets
        .iter()
        .map(|target| {
            received.remove(target.id).unwrap_or(ProviderProbeOutcome {
                id: target.id,
                ok: false,
                latency_ms: 0,
                http_status: None,
                error_code: "probe_deadline_exceeded",
            })
        })
        .collect()
}

fn probe_endpoint_with_client(
    client: &reqwest::blocking::Client,
    id: &'static str,
    endpoint: &str,
) -> ProviderProbeOutcome {
    let started = Instant::now();
    let latency_ms = || started.elapsed().as_millis() as u64;
    match client.head(endpoint).send() {
        Ok(response) => ProviderProbeOutcome {
            id,
            ok: true,
            latency_ms: latency_ms(),
            http_status: Some(response.status().as_u16()),
            error_code: "http_reached",
        },
        Err(error) if error.is_timeout() => ProviderProbeOutcome {
            id,
            ok: false,
            latency_ms: latency_ms(),
            http_status: None,
            error_code: "timeout",
        },
        Err(_) => ProviderProbeOutcome {
            id,
            ok: false,
            latency_ms: latency_ms(),
            http_status: None,
            error_code: "transport_unavailable",
        },
    }
}

pub(crate) fn test_connection_for_settings(
    settings: &crate::ApplicationNetworkSettings,
) -> &'static str {
    use reqwest::blocking::Client;
    use reqwest::redirect::Policy;
    use std::time::Duration;

    let builder = Client::builder()
        .redirect(Policy::none())
        .retry(reqwest::retry::never())
        .cookie_store(false)
        .referer(false)
        .connect_timeout(Duration::from_secs(3))
        .timeout(Duration::from_secs(5));
    let builder = match settings.proxy_mode() {
        crate::NetworkProxyMode::Direct => builder.no_proxy(),
        crate::NetworkProxyMode::Auto => builder,
        crate::NetworkProxyMode::Manual => {
            let Ok(proxy) = reqwest::Proxy::all(&settings.proxy_url) else {
                return "proxy_configuration_rejected";
            };
            builder.no_proxy().proxy(proxy)
        }
    };
    let Ok(client) = builder.build() else {
        return "proxy_configuration_rejected";
    };
    let endpoint = insar_download::CDSE_IDENTITY_TOKEN_ENDPOINT;
    match client.head(endpoint).send() {
        Ok(_) => "http_reached",
        Err(error) if error.is_timeout() => "timeout",
        Err(_) => "transport_unavailable",
    }
}

#[cfg(test)]
mod tests {
    use std::io::{Read, Write};
    use std::net::TcpListener;
    use std::thread;
    use std::time::Duration;

    use super::*;
    use crate::{ApplicationNetworkSettings, NetworkProxyMode};

    #[test]
    fn manual_proxy_connection_probe_uses_configured_host_without_credentials() {
        let listener = TcpListener::bind("127.0.0.1:0").expect("loopback proxy fixture");
        let address = listener.local_addr().unwrap();
        let probe = thread::spawn(move || {
            let (mut stream, _) = listener.accept().expect("probe reaches fixture proxy");
            stream
                .set_read_timeout(Some(Duration::from_secs(2)))
                .unwrap();
            let mut input = [0u8; 1024];
            let size = stream.read(&mut input).expect("probe request");
            let request = String::from_utf8_lossy(&input[..size]);
            assert!(request.starts_with("CONNECT identity.dataspace.copernicus.eu:443 HTTP/1.1"));
            assert!(!request
                .to_ascii_lowercase()
                .contains("proxy-authorization:"));
            stream
                .write_all(b"HTTP/1.1 200 Connection established\r\n\r\n")
                .unwrap();
        });
        let settings = ApplicationNetworkSettings {
            proxy_enabled: true,
            proxy_url: format!("http://{address}"),
            ..ApplicationNetworkSettings::default()
        };
        assert_eq!(settings.proxy_mode(), NetworkProxyMode::Manual);
        let stage = test_connection_for_settings(&settings);
        probe.join().expect("fixture proxy responded");
        assert_ne!(stage, "proxy_configuration_rejected");
    }

    #[test]
    fn saved_settings_select_each_service_route() {
        let manual = ApplicationNetworkSettings {
            proxy_enabled: true,
            proxy_url: "http://127.0.0.1:3128".to_owned(),
            ..ApplicationNetworkSettings::default()
        };
        let auto = ApplicationNetworkSettings {
            proxy_enabled: true,
            ..ApplicationNetworkSettings::default()
        };
        let direct = ApplicationNetworkSettings::default();
        let proxy =
            ProviderNetworkPolicy::manual(insar_core::ManualProxy::new("127.0.0.1", 3128).unwrap());
        assert_eq!(policy_for_settings(ServiceRoute::Ordinary, &manual), proxy);
        assert_eq!(policy_for_settings(ServiceRoute::Bhoonidhi, &manual), proxy);
        assert_eq!(
            policy_for_settings(ServiceRoute::Ordinary, &auto),
            ProviderNetworkPolicy::system()
        );
        assert_eq!(
            policy_for_settings(ServiceRoute::Ordinary, &direct),
            ProviderNetworkPolicy::direct()
        );
        assert_eq!(
            policy_for_settings(ServiceRoute::Bhoonidhi, &direct),
            ProviderNetworkPolicy::system()
        );
        assert_eq!(
            policy_for_settings(ServiceRoute::Bhoonidhi, &auto),
            ProviderNetworkPolicy::system()
        );
    }

    #[test]
    fn saving_a_manual_proxy_rebuilds_idle_sessions_for_new_requests() {
        let dir = std::env::temp_dir().join(format!("w1-network-apply-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let service = crate::ApplicationService::new(
            crate::ApplicationServiceConfig::business_store(dir.join("store.sqlite3")),
        );
        let direct = ProviderNetworkPolicy::direct();
        assert_eq!(
            service.cdse_session_runtime().unwrap().network_policy,
            direct
        );
        assert_eq!(
            service
                .usgs_m2m_session_runtime()
                .unwrap()
                .session
                .network_policy(),
            direct
        );
        service
            .save_network_settings(&ApplicationNetworkSettings {
                proxy_enabled: true,
                proxy_url: "http://127.0.0.1:3128".to_owned(),
                ..ApplicationNetworkSettings::default()
            })
            .unwrap();
        let proxy =
            ProviderNetworkPolicy::manual(insar_core::ManualProxy::new("127.0.0.1", 3128).unwrap());
        assert_eq!(
            service.cdse_session_runtime().unwrap().network_policy,
            proxy
        );
        assert_eq!(
            service
                .usgs_m2m_session_runtime()
                .unwrap()
                .session
                .network_policy(),
            proxy
        );
        assert_eq!(service.network_policy(ServiceRoute::Bhoonidhi), proxy);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn contended_network_policy_save_applies_before_catalogue_gate_acquires_dispatcher() {
        let dir = std::env::temp_dir().join(format!(
            "w1-contended-catalogue-gate-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let service = crate::ApplicationService::new(
            crate::ApplicationServiceConfig::business_store(dir.join("store.sqlite3")),
        );
        service
            .save_network_settings(&ApplicationNetworkSettings {
                proxy_enabled: true,
                proxy_url: "http://127.0.0.1:7890".to_owned(),
                ..ApplicationNetworkSettings::default()
            })
            .unwrap();
        let initial_proxy =
            ProviderNetworkPolicy::manual(insar_core::ManualProxy::new("127.0.0.1", 7890).unwrap());
        let _dispatcher = service.dispatcher_handle().unwrap();
        assert_eq!(
            service.cdse_session_runtime().unwrap().network_policy,
            initial_proxy
        );

        let gate = service.catalogue_policy_gate.lock().unwrap();
        service
            .save_network_settings(&ApplicationNetworkSettings {
                proxy_enabled: true,
                proxy_url: "http://127.0.0.1:7897".to_owned(),
                ..ApplicationNetworkSettings::default()
            })
            .unwrap();
        assert_eq!(
            service.cdse_session_runtime().unwrap().network_policy,
            initial_proxy
        );
        drop(gate);

        service.apply_pending_network_policy();
        let _gate = service.catalogue_policy_gate.lock().unwrap();
        let _dispatcher = service.dispatcher_handle().unwrap();
        let updated_proxy =
            ProviderNetworkPolicy::manual(insar_core::ManualProxy::new("127.0.0.1", 7897).unwrap());
        assert_eq!(
            service.cdse_session_runtime().unwrap().network_policy,
            updated_proxy
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    fn runtime(
        status: insar_core::TaskLifecycleStatus,
        active: &[&str],
        queued: &[&str],
        paused: bool,
    ) -> insar_core::TaskRuntimeSnapshot {
        let ids = |values: &[&str]| values.iter().map(|id| (*id).to_owned()).collect();
        insar_core::TaskRuntimeSnapshot {
            task_id: "task-1".to_owned(),
            status,
            selected_item_ids: vec!["a".to_owned(), "b".to_owned()],
            checked_item_ids: vec!["a".to_owned(), "b".to_owned()],
            page_index: 0,
            page_size: 24,
            active_item_ids: ids(active),
            paused_item_ids: if paused { ids(queued) } else { Vec::new() },
            queued_item_ids: if paused { Vec::new() } else { ids(queued) },
            completed_item_ids: Vec::new(),
            failed_item_ids: Vec::new(),
            skipped_item_ids: Vec::new(),
            cancelled_item_ids: Vec::new(),
            task_paused: paused,
            concurrency: 1,
            revision: 1,
            updated_at: "2026-01-01T00:00:00Z".to_owned(),
        }
    }

    #[test]
    fn a_task_between_items_still_blocks_the_route_change() {
        use insar_core::TaskLifecycleStatus::{Paused, Queued, Running};
        // The scheduler may start the queued item right after the check, and
        // dropping the old runtime would cancel that worker.
        let between_items = runtime(Running, &[], &["b"], false);
        let not_started = runtime(Queued, &[], &["a"], false);
        let transferring = runtime(Running, &["a"], &[], false);
        let paused = runtime(Paused, &[], &["a"], true);
        assert!(runtimes_may_transfer(&[between_items]));
        assert!(runtimes_may_transfer(&[not_started]));
        assert!(runtimes_may_transfer(&[transferring]));
        assert!(!runtimes_may_transfer(&[paused]));
        assert!(!runtimes_may_transfer(&[]));
    }

    #[test]
    fn legacy_cdse_status_reports_a_manual_route_as_system() {
        let manual =
            ProviderNetworkPolicy::manual(insar_core::ManualProxy::new("127.0.0.1", 3128).unwrap());
        // The frozen legacy UI only parses "system" and "direct".
        assert_eq!(
            serde_json::to_value(crate::CdseNetworkMode::from_policy(manual)).unwrap(),
            serde_json::json!("system")
        );
        let legacy = ApplicationNetworkSettings {
            proxy_enabled: true,
            proxy_url: "http://127.0.0.1:3128".to_owned(),
            ..ApplicationNetworkSettings::default()
        };
        assert_eq!(legacy.proxy_mode(), NetworkProxyMode::Manual);
        assert_eq!(policy_for_settings(ServiceRoute::Ordinary, &legacy), manual);
    }

    #[test]
    fn a_legacy_system_choice_keeps_the_saved_manual_route() {
        let dir = std::env::temp_dir().join(format!("w1-legacy-mode-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let service = crate::ApplicationService::new(
            crate::ApplicationServiceConfig::business_store(dir.join("store.sqlite3")),
        );
        service
            .save_network_settings(&ApplicationNetworkSettings {
                proxy_enabled: true,
                proxy_url: "http://127.0.0.1:3128".to_owned(),
                ..ApplicationNetworkSettings::default()
            })
            .unwrap();
        let manual =
            ProviderNetworkPolicy::manual(insar_core::ManualProxy::new("127.0.0.1", 3128).unwrap());
        assert_eq!(
            service.cdse_session_status().unwrap().network_mode,
            crate::CdseNetworkMode::System
        );
        let status = service
            .set_cdse_network_policy(crate::CdseNetworkMode::System)
            .unwrap();
        assert_eq!(status.network_mode, crate::CdseNetworkMode::System);
        assert_eq!(
            service.cdse_session_runtime().unwrap().network_policy,
            manual
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn ordinary_services_are_direct_bhoonidhi_inherits_system_route() {
        assert_eq!(
            default_policy(ServiceRoute::Ordinary),
            ProviderNetworkPolicy::direct()
        );
        assert_eq!(
            default_policy(ServiceRoute::Bhoonidhi),
            ProviderNetworkPolicy::system()
        );
    }

    #[test]
    fn saving_the_automatic_system_proxy_mode_round_trips_through_the_sidecar() {
        let dir = std::env::temp_dir().join(format!("w1-auto-proxy-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let service = crate::ApplicationService::new(
            crate::ApplicationServiceConfig::business_store(dir.join("store.sqlite3")),
        );
        // The automatic system-proxy mode is saved as an enabled proxy with an
        // empty URL; this save used to be rejected by URL validation.
        let auto = crate::ApplicationNetworkSettings {
            proxy_enabled: true,
            proxy_url: String::new(),
            ..crate::ApplicationNetworkSettings::default()
        };
        let saved = service.save_network_settings(&auto).unwrap();
        assert_eq!(saved, auto);
        // A fresh service on the same sidecar stands in for an app restart:
        // the settings it loads build system-proxy clients, including the
        // Bhoonidhi route this mode exists for.
        let restarted = crate::ApplicationService::new(
            crate::ApplicationServiceConfig::business_store(dir.join("store.sqlite3")),
        );
        assert_eq!(restarted.get_network_settings().unwrap(), auto);
        assert_eq!(
            restarted.network_policy(ServiceRoute::Ordinary),
            ProviderNetworkPolicy::system()
        );
        assert_eq!(
            restarted.network_policy(ServiceRoute::Bhoonidhi),
            ProviderNetworkPolicy::system()
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn probe_targets_cover_every_listed_provider_with_https_sites() {
        let targets = provider_probe_targets();
        let ids: Vec<&str> = targets.iter().map(|target| target.id).collect();
        assert_eq!(
            ids,
            vec![
                "cdse",
                "earthdata",
                "bhoonidhi",
                "usgs_m2m",
                "mpc",
                "opentopography",
                "gee",
                "gacos",
                "comet_licsar",
            ]
        );
        for target in &targets {
            if target.id == "gacos" {
                // GACOS official portal (www.gacos.net) serves HTTP only.
                assert_eq!(
                    target.endpoint.as_ref(),
                    insar_provider::gacos_delivery::GACOS_WEB_ROOT
                );
                assert_eq!(target.site, insar_provider::gacos_delivery::GACOS_WEB_ROOT);
            } else {
                assert!(target.endpoint.starts_with("https://"), "{}", target.id);
                assert!(target.site.starts_with("https://"), "{}", target.id);
            }
            assert_eq!(provider_probe_site(target.id), Some(target.site));
        }
        assert_eq!(provider_probe_site("unknown"), None);
    }

    #[test]
    fn a_rejected_manual_proxy_reports_every_endpoint_as_rejected() {
        let targets = provider_probe_targets();
        // A control character makes the URL unparsable for reqwest's proxy
        // scheme without any network access.
        let settings = crate::ApplicationNetworkSettings {
            proxy_enabled: true,
            proxy_url: "http://bad\u{0}proxy:7890".to_owned(),
            ..crate::ApplicationNetworkSettings::default()
        };
        let outcomes = probe_targets_with_settings(&targets, &settings);
        assert_eq!(outcomes.len(), targets.len());
        assert!(outcomes.iter().all(|outcome| !outcome.ok
            && outcome.http_status.is_none()
            && outcome.latency_ms == 0
            && outcome.error_code == "proxy_configuration_rejected"));
    }

    #[test]
    fn probing_reports_each_loopback_endpoint_with_latency_or_reason() {
        // One green loopback endpoint and one refused port cover both probe
        // result shapes without contacting any real provider.
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let green = listener.local_addr().unwrap();
        let responder = thread::spawn(move || {
            if let Ok((mut stream, _)) = listener.accept() {
                let mut input = [0u8; 512];
                let _ = stream.read(&mut input);
                let _ = stream.write_all(
                    b"HTTP/1.1 204 No Content\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
                );
            }
        });
        let refused_listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let refused = refused_listener.local_addr().unwrap();
        drop(refused_listener);
        let targets = vec![
            ProviderProbeTarget {
                id: "green",
                endpoint: format!("http://{green}/").into(),
                site: "https://example.invalid/",
            },
            ProviderProbeTarget {
                id: "refused",
                endpoint: format!("http://{refused}/").into(),
                site: "https://example.invalid/",
            },
        ];
        let settings = ApplicationNetworkSettings::default();
        let outcomes = probe_targets_with_settings(&targets, &settings);
        assert_eq!(
            outcomes
                .iter()
                .map(|outcome| outcome.id)
                .collect::<Vec<_>>(),
            vec!["green", "refused"]
        );
        let green_outcome = &outcomes[0];
        assert!(green_outcome.ok, "loopback responder must probe green");
        assert_eq!(green_outcome.error_code, "http_reached");
        assert_eq!(green_outcome.http_status, Some(204));
        let refused_outcome = &outcomes[1];
        assert!(!refused_outcome.ok);
        assert_eq!(refused_outcome.error_code, "transport_unavailable");
        assert!(refused_outcome.http_status.is_none());
        responder.join().expect("loopback responder finished");
    }

    #[test]
    fn probe_policy_matches_each_providers_real_client_route() {
        let direct = ApplicationNetworkSettings::default();
        // Direct mode: ordinary providers go direct, but Bhoonidhi's real
        // client inherits the system proxy, so its probe must too.
        assert_eq!(
            probe_policy_for("cdse", &direct),
            insar_core::ProxyPolicy::Direct
        );
        assert_eq!(
            probe_policy_for("bhoonidhi", &direct),
            insar_core::ProxyPolicy::System
        );
        for id in [
            "earthdata",
            "usgs_m2m",
            "mpc",
            "opentopography",
            "gee",
            "gacos",
            "comet_licsar",
        ] {
            assert_eq!(
                probe_policy_for(id, &direct),
                insar_core::ProxyPolicy::Direct,
                "{id}"
            );
        }
        // Every mode keeps the probe on the same route the real client uses.
        let auto = ApplicationNetworkSettings {
            proxy_enabled: true,
            ..ApplicationNetworkSettings::default()
        };
        assert_eq!(
            probe_policy_for("bhoonidhi", &auto),
            insar_core::ProxyPolicy::System
        );
        assert_eq!(
            probe_policy_for("cdse", &auto),
            insar_core::ProxyPolicy::System
        );
        let manual = ApplicationNetworkSettings {
            proxy_enabled: true,
            proxy_url: "http://127.0.0.1:3128".to_owned(),
            ..ApplicationNetworkSettings::default()
        };
        let expected = insar_core::ProxyPolicy::Manual(
            insar_core::ManualProxy::new("127.0.0.1", 3128).unwrap(),
        );
        assert_eq!(probe_policy_for("bhoonidhi", &manual), expected);
        assert_eq!(probe_policy_for("cdse", &manual), expected);
    }

    #[test]
    fn direct_mode_probe_builds_bhoonidhi_on_the_system_route_not_no_proxy() {
        // In Direct mode the Bhoonidhi probe must follow the same system route
        // its real client uses instead of the no_proxy() bypass the ordinary
        // providers get. reqwest resolves and caches the concrete system proxy
        // once per process, so the contract is pinned at the policy seam the
        // probe's client builder consumes.
        let direct = ApplicationNetworkSettings::default();
        assert_eq!(direct.proxy_mode(), NetworkProxyMode::Direct);
        assert_eq!(
            probe_policy_for("bhoonidhi", &direct),
            insar_core::ProxyPolicy::System,
            "Direct mode must not bypass the system proxy for Bhoonidhi"
        );
        assert_ne!(
            probe_policy_for("bhoonidhi", &direct),
            probe_policy_for("cdse", &direct),
            "the Bhoonidhi probe route must differ from the ordinary direct route"
        );
    }

    #[test]
    fn a_hung_probe_worker_yields_an_explicit_failed_entry() {
        // A worker that never sends (here: an unroutable endpoint that cannot
        // connect) still yields an entry; the overall deadline bounds the whole
        // probe so a panicking or hung worker is reported as failed, never
        // omitted.
        let targets = vec![ProviderProbeTarget {
            id: "stalled",
            endpoint: "http://127.0.0.1:1/".into(),
            site: "https://example.invalid/",
        }];
        let settings = ApplicationNetworkSettings::default();
        let outcomes = probe_targets_with_settings(&targets, &settings);
        assert_eq!(outcomes.len(), 1);
        assert!(!outcomes[0].ok);
        assert!(
            outcomes[0].error_code == "transport_unavailable"
                || outcomes[0].error_code == "probe_deadline_exceeded"
        );
        assert!(outcomes[0].http_status.is_none());
    }

    #[test]
    fn apply_network_policy_if_idle_defers_on_corrupt_settings_and_succeeds_without_store_path() {
        let liveness =
            crate::ApplicationService::new(crate::ApplicationServiceConfig::liveness_only());
        assert_eq!(
            liveness.apply_network_policy_if_idle(),
            ApplyOutcome::Applied
        );

        let root = std::env::temp_dir().join(format!(
            "insar-network-policy-corrupt-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&root).unwrap();
        let store_path = root.join("tasks.sqlite3");
        let _store = insar_store::TaskStore::open(&store_path).unwrap();
        std::fs::write(root.join("network-settings.json"), b"{not valid json").unwrap();

        let service = crate::ApplicationService::new(
            crate::ApplicationServiceConfig::business_store(&store_path),
        );
        service
            .network_apply
            .requested
            .fetch_add(1, Ordering::AcqRel);
        assert_eq!(service.apply_network_policy_if_idle(), ApplyOutcome::Busy);
        assert_eq!(service.network_apply.applied.load(Ordering::Acquire), 0);

        let _ = std::fs::remove_dir_all(&root);
    }
}
