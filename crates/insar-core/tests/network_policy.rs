use insar_core::{
    dns_name_matches, normalize_dns_name, ManualProxy, ProviderNetworkPolicy, ProxyPolicy,
};

#[test]
fn provider_route_policy_is_bounded_and_defaults_to_system() {
    assert_eq!(
        ProviderNetworkPolicy::default(),
        ProviderNetworkPolicy::system()
    );
    assert_eq!(ProviderNetworkPolicy::system().route(), ProxyPolicy::System);
    assert_eq!(ProviderNetworkPolicy::direct().route(), ProxyPolicy::Direct);
}

#[test]
fn dns_suffix_matching_uses_complete_normalized_labels() {
    assert!(dns_name_matches(
        "earthdatacloud.nasa.gov",
        "earthdatacloud.nasa.gov"
    ));
    assert!(dns_name_matches(
        "bucket.earthdatacloud.nasa.gov.",
        "earthdatacloud.nasa.gov"
    ));
    assert!(!dns_name_matches(
        "fakeearthdatacloud.nasa.gov",
        "earthdatacloud.nasa.gov"
    ));
    assert!(!dns_name_matches(
        "earthdatacloud.nasa.gov.evil.example",
        "earthdatacloud.nasa.gov"
    ));
}

#[test]
fn dns_normalization_rejects_invalid_or_ambiguous_names() {
    assert_eq!(
        normalize_dns_name(" EarthDataCloud.NASA.GOV. ").unwrap(),
        "earthdatacloud.nasa.gov"
    );
    for invalid in [
        "",
        ".",
        "double..dot",
        "-bad.example",
        "bad-.example",
        "bad_name",
    ] {
        assert!(normalize_dns_name(invalid).is_err(), "{invalid}");
    }
}

#[test]
fn manual_proxy_is_a_credential_free_host_and_explicit_port() {
    let proxy = ManualProxy::new(" Proxy.Example. ", 8080).expect("dns proxy");
    assert_eq!(proxy.host(), "proxy.example");
    assert_eq!(proxy.url(), "http://proxy.example:8080");
    let policy = ProviderNetworkPolicy::manual(proxy);
    assert_eq!(policy.route(), ProxyPolicy::Manual(proxy));
    assert_eq!(
        ManualProxy::new("[::1]", 3128).expect("ipv6 proxy").url(),
        "http://[::1]:3128"
    );
    assert_eq!(
        ManualProxy::new("127.0.0.1", 9).expect("ipv4 proxy").url(),
        "http://127.0.0.1:9"
    );
    for (host, port) in [
        ("proxy.example", 0),
        ("user:pass@proxy.example", 8080),
        ("http://proxy.example", 8080),
        ("proxy.example/path", 8080),
        ("", 8080),
    ] {
        assert!(ManualProxy::new(host, port).is_err(), "{host}:{port}");
    }
}
