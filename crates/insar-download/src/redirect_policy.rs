//! Unified HTTP redirect host-allowlist and SSRF validation for `insar-download`.

use insar_core::dns_name_matches;
use reqwest::Url;
use std::net::{IpAddr, Ipv4Addr};

/// Explicitly allowed public science, provider, and cloud object-storage domain
/// suffixes when following redirects for unpolicied or generic transfer paths.
const PUBLIC_REDIRECT_ALLOWLIST_DOMAINS: &[&str] = &[
    "earthdata.nasa.gov",
    "earthdatacloud.nasa.gov",
    "eosdis.nasa.gov",
    "nasa.gov",
    "asf.alaska.edu",
    "amazonaws.com",
    "cloudfront.net",
    "blob.core.windows.net",
    "storage.googleapis.com",
    "dataspace.copernicus.eu",
    "copernicus.eu",
    "usgs.gov",
    "nrsc.gov.in",
    "isro.gov.in",
    "esa.int",
    "gacos.net",
    "leeds.ac.uk",
    "jasmin.ac.uk",
    "nerc.ac.uk",
    "dlr.de",
    "opentopography.org",
    "geoboundaries.org",
    "unr.edu",
    "github.com",
    "githubusercontent.com",
];

const EARTHDATA_REDIRECT_ALLOWLIST_DOMAINS: &[&str] = &[
    "earthdata.nasa.gov",
    "earthdatacloud.nasa.gov",
    "eosdis.nasa.gov",
    "nasa.gov",
    "asf.alaska.edu",
    "amazonaws.com",
    "cloudfront.net",
];

const BHOONIDHI_ALLOWLIST_DOMAINS: &[&str] = &["nrsc.gov.in", "isro.gov.in"];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum NisarRangeRedirectAuth {
    None,
    Earthdata,
    Bhoonidhi,
}

fn normalized_domain(domain: &str) -> String {
    domain.trim_end_matches('.').to_ascii_lowercase()
}

pub(crate) fn dns_domain_matches(host: &str, allowed_suffix: &str) -> bool {
    let host = normalized_domain(host);
    let suffix = normalized_domain(allowed_suffix);
    if host.is_empty() || suffix.is_empty() {
        return false;
    }
    dns_name_matches(&host, &suffix)
}

fn parse_url_ip(url: &Url) -> Option<IpAddr> {
    let raw = url.host_str()?;
    let stripped = raw
        .strip_prefix('[')
        .and_then(|s| s.strip_suffix(']'))
        .unwrap_or(raw);
    stripped.parse::<IpAddr>().ok()
}

fn host_equals_normalized(a: &Url, b: &Url) -> bool {
    match (parse_url_ip(a), parse_url_ip(b)) {
        (Some(ia), Some(ib)) => ia == ib,
        (None, None) => match (a.host_str(), b.host_str()) {
            (Some(da), Some(db)) => normalized_domain(da) == normalized_domain(db),
            _ => false,
        },
        _ => false,
    }
}

pub(crate) fn same_origin(a: &Url, b: &Url) -> bool {
    a.scheme() == b.scheme()
        && host_equals_normalized(a, b)
        && a.port_or_known_default() == b.port_or_known_default()
}

pub(crate) fn is_loopback_url(url: &Url) -> bool {
    if let Some(ip) = parse_url_ip(url) {
        return match ip {
            IpAddr::V4(v4) => v4.is_loopback(),
            IpAddr::V6(v6) => {
                v6.is_loopback() || v6.to_ipv4_mapped().is_some_and(|v4| v4.is_loopback())
            }
        };
    }
    url.host_str()
        .is_some_and(|d| normalized_domain(d) == "localhost")
}

fn is_disallowed_ipv4(ip: Ipv4Addr) -> bool {
    let [o0, o1, ..] = ip.octets();
    ip.is_private()
        || ip.is_loopback()
        || ip.is_link_local()
        || ip.is_broadcast()
        || ip.is_documentation()
        || ip.is_multicast()
        || ip.is_unspecified()
        || o0 == 0
        || (o0 == 100 && (64..=127).contains(&o1))
        || (o0 == 198 && (18..=19).contains(&o1))
        || o0 >= 240
}

pub(crate) fn is_disallowed_non_public_host(url: &Url) -> bool {
    if let Some(ip) = parse_url_ip(url) {
        return match ip {
            IpAddr::V4(v4) => is_disallowed_ipv4(v4),
            IpAddr::V6(v6) => {
                v6.is_loopback()
                    || v6.is_unspecified()
                    || v6.is_unique_local()
                    || v6.is_unicast_link_local()
                    || v6.is_multicast()
                    || v6.to_ipv4_mapped().is_some_and(is_disallowed_ipv4)
            }
        };
    }
    let Some(d) = url.host_str() else {
        return true;
    };
    let norm = normalized_domain(d);
    norm.is_empty()
        || !norm.contains('.')
        || norm == "localhost"
        || norm.ends_with(".localhost")
        || norm.ends_with(".local")
        || norm.ends_with(".internal")
}

fn redirect_base_checks(initial: &Url, candidate: &Url) -> bool {
    if !matches!(initial.scheme(), "http" | "https")
        || !matches!(candidate.scheme(), "http" | "https")
    {
        return false;
    }
    if initial.scheme() == "https" && candidate.scheme() != "https" {
        return false;
    }
    if !candidate.username().is_empty() || candidate.password().is_some() {
        return false;
    }
    true
}

fn candidate_uses_standard_port(candidate: &Url) -> bool {
    let expected_port = match candidate.scheme() {
        "https" => Some(443),
        "http" => Some(80),
        _ => None,
    };
    candidate.port_or_known_default() == expected_port
}

fn candidate_domain_in(candidate: &Url, allowlist: &[&str]) -> bool {
    if parse_url_ip(candidate).is_some() {
        return false;
    }
    let Some(domain) = candidate.host_str() else {
        return false;
    };
    allowlist
        .iter()
        .any(|allowed| dns_domain_matches(domain, allowed))
}

pub(crate) fn default_redirect_target_allowed(initial: &Url, candidate: &Url) -> bool {
    if !redirect_base_checks(initial, candidate) {
        return false;
    }
    if is_loopback_url(initial) {
        return is_loopback_url(candidate);
    }
    if is_disallowed_non_public_host(candidate) || !candidate_uses_standard_port(candidate) {
        return false;
    }
    host_equals_normalized(initial, candidate)
        || candidate_domain_in(candidate, PUBLIC_REDIRECT_ALLOWLIST_DOMAINS)
}

fn is_standard_https(url: &Url) -> bool {
    url.scheme() == "https" && url.port_or_known_default() == Some(443)
}

pub(crate) fn is_bhoonidhi_host(url: &Url) -> bool {
    is_standard_https(url) && candidate_domain_in(url, BHOONIDHI_ALLOWLIST_DOMAINS)
}

pub(crate) fn is_earthdata_auth_host(url: &Url) -> bool {
    is_standard_https(url)
        && candidate_domain_in(
            url,
            &[
                "earthdata.nasa.gov",
                "earthdatacloud.nasa.gov",
                "eosdis.nasa.gov",
                "nasa.gov",
                "asf.alaska.edu",
            ],
        )
}

pub(crate) fn nisar_range_redirect_allowed(
    initial: &Url,
    candidate: &Url,
    auth: NisarRangeRedirectAuth,
) -> bool {
    match auth {
        NisarRangeRedirectAuth::None => default_redirect_target_allowed(initial, candidate),
        NisarRangeRedirectAuth::Bhoonidhi => {
            if !redirect_base_checks(initial, candidate) {
                return false;
            }
            if is_loopback_url(initial) {
                return is_loopback_url(candidate);
            }
            !is_disallowed_non_public_host(candidate)
                && candidate_uses_standard_port(candidate)
                && candidate_domain_in(candidate, BHOONIDHI_ALLOWLIST_DOMAINS)
        }
        NisarRangeRedirectAuth::Earthdata => {
            if !redirect_base_checks(initial, candidate) {
                return false;
            }
            if is_loopback_url(initial) {
                return is_loopback_url(candidate);
            }
            !is_disallowed_non_public_host(candidate)
                && candidate_uses_standard_port(candidate)
                && (host_equals_normalized(initial, candidate)
                    || candidate_domain_in(candidate, EARTHDATA_REDIRECT_ALLOWLIST_DOMAINS))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn public_https_rejects_ssrf_downgrade_userinfo_and_unknown_hosts() {
        let initial = Url::parse("https://datapool.asf.alaska.edu/file.zip").unwrap();
        assert!(!default_redirect_target_allowed(
            &initial,
            &Url::parse("https://127.0.0.1/secret").unwrap()
        ));
        assert!(!default_redirect_target_allowed(
            &initial,
            &Url::parse("https://169.254.169.254/latest/meta-data/").unwrap()
        ));
        assert!(!default_redirect_target_allowed(
            &initial,
            &Url::parse("https://10.0.0.1/internal").unwrap()
        ));
        assert!(!default_redirect_target_allowed(
            &initial,
            &Url::parse("https://100.64.0.1/cgnat").unwrap()
        ));
        assert!(!default_redirect_target_allowed(
            &initial,
            &Url::parse("https://[::1]/loopback").unwrap()
        ));
        assert!(!default_redirect_target_allowed(
            &initial,
            &Url::parse("https://[::ffff:127.0.0.1]/mapped").unwrap()
        ));
        assert!(!default_redirect_target_allowed(
            &initial,
            &Url::parse("https://[::ffff:169.254.169.254]/mapped-meta").unwrap()
        ));
        assert!(!default_redirect_target_allowed(
            &initial,
            &Url::parse("https://metadata.google.internal/computeMetadata").unwrap()
        ));
        assert!(!default_redirect_target_allowed(
            &initial,
            &Url::parse("https://nas.local/share").unwrap()
        ));
        assert!(!default_redirect_target_allowed(
            &initial,
            &Url::parse("http://datapool.asf.alaska.edu/downgrade.zip").unwrap()
        ));
        assert!(!default_redirect_target_allowed(
            &initial,
            &Url::parse("https://user:pass@d1bctmng8vmwga.cloudfront.net/file.zip").unwrap()
        ));
        assert!(!default_redirect_target_allowed(
            &initial,
            &Url::parse("https://evil.example.com/payload.zip").unwrap()
        ));
        assert!(!default_redirect_target_allowed(
            &initial,
            &Url::parse("https://cloudfront.net:8443/nonstd-port.zip").unwrap()
        ));
        assert!(default_redirect_target_allowed(
            &initial,
            &Url::parse("https://urs.earthdata.nasa.gov/oauth/authorize").unwrap()
        ));
        assert!(default_redirect_target_allowed(
            &initial,
            &Url::parse("https://d1bctmng8vmwga.cloudfront.net/file.zip").unwrap()
        ));
    }

    #[test]
    fn http_initial_still_blocks_ssrf_but_allows_gacos_https_upgrade() {
        let gacos = Url::parse("http://www.gacos.net/M/example.tar.gz").unwrap();
        assert!(default_redirect_target_allowed(
            &gacos,
            &Url::parse("https://www.gacos.net/M/example.tar.gz").unwrap()
        ));
        assert!(!default_redirect_target_allowed(
            &gacos,
            &Url::parse("http://127.0.0.1:8080/steal").unwrap()
        ));
        assert!(!default_redirect_target_allowed(
            &gacos,
            &Url::parse("http://192.168.1.10/router").unwrap()
        ));
        assert!(!default_redirect_target_allowed(
            &gacos,
            &Url::parse("http://metadata.google.internal/").unwrap()
        ));
    }

    #[test]
    fn loopback_initial_allows_cross_port_loopback_and_blocks_external() {
        let local_http = Url::parse("http://127.0.0.1:18080/a").unwrap();
        assert!(default_redirect_target_allowed(
            &local_http,
            &Url::parse("http://127.0.0.1:19090/b").unwrap()
        ));
        assert!(default_redirect_target_allowed(
            &local_http,
            &Url::parse("http://[::1]:19090/b").unwrap()
        ));
        assert!(default_redirect_target_allowed(
            &local_http,
            &Url::parse("http://localhost:19090/b").unwrap()
        ));
        assert!(!default_redirect_target_allowed(
            &local_http,
            &Url::parse("https://evil.example.com/c").unwrap()
        ));
    }

    #[test]
    fn nisar_range_redirect_respects_per_provider_boundaries() {
        let bhoonidhi = Url::parse("https://bhoonidhi.nrsc.gov.in/data.h5").unwrap();
        assert!(nisar_range_redirect_allowed(
            &bhoonidhi,
            &Url::parse("https://cdn.isro.gov.in/data.h5").unwrap(),
            NisarRangeRedirectAuth::Bhoonidhi
        ));
        assert!(!nisar_range_redirect_allowed(
            &bhoonidhi,
            &Url::parse("https://evil.amazonaws.com/data.h5").unwrap(),
            NisarRangeRedirectAuth::Bhoonidhi
        ));

        let edl = Url::parse("https://nisar.asf.earthdatacloud.nasa.gov/granule.h5").unwrap();
        assert!(nisar_range_redirect_allowed(
            &edl,
            &Url::parse("https://urs.earthdata.nasa.gov/oauth/authorize").unwrap(),
            NisarRangeRedirectAuth::Earthdata
        ));
        assert!(nisar_range_redirect_allowed(
            &edl,
            &Url::parse("https://s3.us-west-2.amazonaws.com/bucket/granule.h5").unwrap(),
            NisarRangeRedirectAuth::Earthdata
        ));
        assert!(!nisar_range_redirect_allowed(
            &edl,
            &Url::parse("https://evil.example.com/granule.h5").unwrap(),
            NisarRangeRedirectAuth::Earthdata
        ));

        assert!(is_bhoonidhi_host(
            &Url::parse("https://bhoonidhi-api.nrsc.gov.in/download").unwrap()
        ));
        assert!(!is_bhoonidhi_host(
            &Url::parse("http://bhoonidhi-api.nrsc.gov.in/download").unwrap()
        ));
        assert!(!is_bhoonidhi_host(
            &Url::parse("https://bhoonidhi-api.nrsc.gov.in:8443/download").unwrap()
        ));
        assert!(is_earthdata_auth_host(
            &Url::parse("https://urs.earthdata.nasa.gov/oauth/authorize").unwrap()
        ));
        assert!(!is_earthdata_auth_host(
            &Url::parse("http://urs.earthdata.nasa.gov/oauth/authorize").unwrap()
        ));
        assert!(!is_earthdata_auth_host(
            &Url::parse("https://urs.earthdata.nasa.gov:8443/oauth/authorize").unwrap()
        ));
    }
}
