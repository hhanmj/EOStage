//! The one place that turns a provider network policy into reqwest proxy
//! settings. Every provider and transfer client builder goes through here so
//! a saved manual proxy reaches all ordinary data-source requests.

use insar_core::{ManualProxy, ProxyPolicy};

pub fn blocking_builder_with_proxy(
    builder: reqwest::blocking::ClientBuilder,
    policy: ProxyPolicy,
) -> reqwest::blocking::ClientBuilder {
    match policy {
        ProxyPolicy::System => builder,
        ProxyPolicy::Direct => builder.no_proxy(),
        ProxyPolicy::Manual(proxy) => builder.no_proxy().proxy(manual_proxy(proxy)),
    }
}

pub fn async_builder_with_proxy(
    builder: reqwest::ClientBuilder,
    policy: ProxyPolicy,
) -> reqwest::ClientBuilder {
    match policy {
        ProxyPolicy::System => builder,
        ProxyPolicy::Direct => builder.no_proxy(),
        ProxyPolicy::Manual(proxy) => builder.no_proxy().proxy(manual_proxy(proxy)),
    }
}

fn manual_proxy(proxy: ManualProxy) -> reqwest::Proxy {
    // ManualProxy only holds a validated DNS name or IP literal and a nonzero
    // port, so the URL always parses.
    reqwest::Proxy::all(proxy.url()).expect("validated manual proxy authority forms a URL")
}

#[cfg(test)]
pub(crate) mod loopback {
    use std::io::{Read, Write};
    use std::net::{SocketAddr, TcpListener};
    use std::thread::{self, JoinHandle};
    use std::time::{Duration, Instant};

    /// Accept one connection within three seconds, answer with `response`
    /// and return the request line. `None` means nothing connected.
    pub(crate) fn serve_once(response: &'static str) -> (SocketAddr, JoinHandle<Option<String>>) {
        let listener = TcpListener::bind("127.0.0.1:0").expect("loopback fixture");
        let address = listener.local_addr().unwrap();
        listener.set_nonblocking(true).unwrap();
        let handle = thread::spawn(move || {
            let deadline = Instant::now() + Duration::from_secs(3);
            loop {
                match listener.accept() {
                    Ok((mut stream, _)) => {
                        stream.set_nonblocking(false).unwrap();
                        stream
                            .set_read_timeout(Some(Duration::from_secs(2)))
                            .unwrap();
                        let mut input = [0u8; 2048];
                        let size = stream.read(&mut input).unwrap_or(0);
                        let _ = stream.write_all(response.as_bytes());
                        let request = String::from_utf8_lossy(&input[..size]).into_owned();
                        return request.lines().next().map(str::to_owned);
                    }
                    Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                        if Instant::now() > deadline {
                            return None;
                        }
                        thread::sleep(Duration::from_millis(10));
                    }
                    Err(_) => return None,
                }
            }
        });
        (address, handle)
    }
}

#[cfg(test)]
mod tests {
    use super::loopback::serve_once;
    use super::*;
    use std::time::Duration;

    const NOT_FOUND: &str =
        "HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n";
    const BAD_GATEWAY: &str =
        "HTTP/1.1 502 Bad Gateway\r\nContent-Length: 0\r\nConnection: close\r\n\r\n";

    #[test]
    fn manual_policy_sends_ordinary_requests_through_the_configured_proxy() {
        let (origin, origin_seen) = serve_once(NOT_FOUND);
        let (proxy, proxy_seen) = serve_once(BAD_GATEWAY);
        let policy =
            ProxyPolicy::Manual(ManualProxy::new(&proxy.ip().to_string(), proxy.port()).unwrap());
        let client = blocking_builder_with_proxy(
            reqwest::blocking::Client::builder()
                .timeout(Duration::from_secs(5))
                .tls_built_in_root_certs(false),
            policy,
        )
        .build()
        .unwrap();
        let response = client.get(format!("http://{origin}/probe")).send();
        let proxy_line = proxy_seen.join().unwrap();
        let _ = origin_seen.join();
        assert_eq!(
            proxy_line.as_deref(),
            Some(format!("GET http://{origin}/probe HTTP/1.1").as_str()),
            "request must reach the manual proxy"
        );
        assert_eq!(response.unwrap().status().as_u16(), 502);
    }

    #[test]
    fn orbit_resolver_built_from_a_manual_policy_resolves_through_the_proxy() {
        use crate::orbit_download::{OrbitHttpClient, OrbitResolverEndpoint};
        let (origin, origin_seen) = serve_once(NOT_FOUND);
        let (proxy, proxy_seen) = serve_once(BAD_GATEWAY);
        let endpoint =
            OrbitResolverEndpoint::loopback_for_tests(&format!("http://{origin}/scene/")).unwrap();
        let client = OrbitHttpClient::with_test_endpoint_and_network_policy(
            endpoint,
            insar_core::ProviderNetworkPolicy::manual(
                ManualProxy::new(&proxy.ip().to_string(), proxy.port()).unwrap(),
            ),
        )
        .unwrap();
        let scene = "S1A_IW_SLC__1SDV_20200101T000000_20200101T000025_030000_036000_ABCD";
        assert!(client.resolve(scene, None, &|| false).is_err());
        assert_eq!(
            proxy_seen.join().unwrap(),
            Some(format!("GET http://{origin}/scene/{scene} HTTP/1.1"))
        );
        let _ = origin_seen.join();
    }

    #[test]
    fn direct_policy_never_uses_a_proxy() {
        let (origin, origin_seen) = serve_once(NOT_FOUND);
        let client = blocking_builder_with_proxy(
            reqwest::blocking::Client::builder()
                .timeout(Duration::from_secs(5))
                .tls_built_in_root_certs(false),
            ProxyPolicy::Direct,
        )
        .build()
        .unwrap();
        let status = client
            .get(format!("http://{origin}/probe"))
            .send()
            .unwrap()
            .status();
        assert_eq!(status.as_u16(), 404);
        assert_eq!(
            origin_seen.join().unwrap().as_deref(),
            Some("GET /probe HTTP/1.1")
        );
    }
}
