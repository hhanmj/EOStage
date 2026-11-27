//! Bounded, credential-free comparison for an explicitly authorized field check.
use super::*;

#[test]
fn declares_windows_system_proxy_for_auth_and_catalogue() {
    for manifest in [
        include_str!("../Cargo.toml"),
        include_str!("../../insar-provider/Cargo.toml"),
    ] {
        let dependency = manifest
            .lines()
            .find(|line| line.starts_with("reqwest = "))
            .unwrap();
        assert!(dependency.contains("\"system-proxy\""), "System policy requires explicit reqwest system-proxy when default features are disabled");
    }
}

fn status_without_credentials(label: &str, client: &Client) -> bool {
    let started = std::time::Instant::now();
    match client.get(BHOONIDHI_TOKEN_ENDPOINT).send() {
        Ok(response) => {
            let status = response.status();
            println!(
                "route={label} HTTP={} elapsed_ms={} auth_sent=false",
                status.as_u16(),
                started.elapsed().as_millis()
            );
            status == StatusCode::UNAUTHORIZED
        }
        Err(error) => {
            println!(
                "route={label} HTTP=0 elapsed_ms={} timeout={} connect={} auth_sent=false",
                started.elapsed().as_millis(),
                error.is_timeout(),
                error.is_connect()
            );
            false
        }
    }
}

#[test]
#[ignore = "explicit fixed-origin anonymous route comparison; no saved credentials"]
fn system_proxy_matches_explicit_bhoonidhi_route() {
    assert_eq!(
        std::env::var("INSAR_BHOONIDHI_ANONYMOUS_ROUTE_CHECK").as_deref(),
        Ok("1")
    );
    let native = ReqwestBhoonidhiTokenEndpoint::new(ProviderNetworkPolicy::system()).unwrap();
    let system_ok = status_without_credentials("production_system", &native.client);
    // This observed local proxy is a test comparison only, never a production default.
    let explicit = Client::builder()
        .redirect(Policy::none())
        .retry(reqwest::retry::never())
        .cookie_store(false)
        .referer(false)
        .connect_timeout(IDENTITY_CONNECT_TIMEOUT)
        .timeout(IDENTITY_CONTROL_TIMEOUT)
        .no_proxy()
        .proxy(reqwest::Proxy::all("http://127.0.0.1:7897").unwrap())
        .build()
        .unwrap();
    let explicit_ok = status_without_credentials("observed_explicit_proxy", &explicit);
    assert!(
        explicit_ok,
        "observed explicit route must be reachable for comparison"
    );
    // HTTP success establishes reachability only; the feature regression above
    // checks Windows proxy support. A direct route can also return HTTP 401.
    assert!(system_ok, "current production route must be reachable");
}
