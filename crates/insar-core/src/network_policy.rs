/// Selects whether a provider HTTP client inherits the operating-system proxy
/// configuration, deliberately bypasses it, or uses one user-configured HTTP
/// proxy. This is runtime-only policy and is not part of persisted task or
/// protocol data.
// The inline manual authority keeps the policy `Copy` for the many client
// constructors that pass it by value; policies are built rarely, not per request.
#[allow(clippy::large_enum_variant)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ProxyPolicy {
    /// Use the operating-system proxy and network route configuration.
    #[default]
    System,
    /// Bypass system proxies. Callers must opt in explicitly.
    Direct,
    /// Send every request through one credential-free HTTP proxy.
    Manual(ManualProxy),
}

const MAX_PROXY_HOST_BYTES: usize = 253;

/// A validated, credential-free HTTP proxy authority (host and explicit port).
/// Stored inline so policies stay `Copy` across provider construction.
#[derive(Clone, Copy, PartialEq, Eq)]
pub struct ManualProxy {
    host: [u8; MAX_PROXY_HOST_BYTES],
    host_len: u8,
    port: u16,
}

impl ManualProxy {
    /// Accepts a DNS name or an IP literal (IPv6 optionally bracketed) and a
    /// nonzero port. Schemes, user info and paths cannot be represented.
    pub fn new(host: &str, port: u16) -> Result<Self, &'static str> {
        if port == 0 {
            return Err("invalid_proxy_port");
        }
        let trimmed = host.trim();
        let unbracketed = trimmed
            .strip_prefix('[')
            .and_then(|value| value.strip_suffix(']'))
            .unwrap_or(trimmed);
        let normalized = match unbracketed.parse::<std::net::IpAddr>() {
            Ok(address) => address.to_string(),
            Err(_) => normalize_dns_name(trimmed).map_err(|_| "invalid_proxy_host")?,
        };
        if normalized.len() > MAX_PROXY_HOST_BYTES {
            return Err("invalid_proxy_host");
        }
        let mut bytes = [0u8; MAX_PROXY_HOST_BYTES];
        bytes[..normalized.len()].copy_from_slice(normalized.as_bytes());
        Ok(Self {
            host: bytes,
            host_len: normalized.len() as u8,
            port,
        })
    }

    pub fn host(&self) -> &str {
        std::str::from_utf8(&self.host[..usize::from(self.host_len)])
            .expect("manual proxy host is validated ASCII")
    }

    pub const fn port(&self) -> u16 {
        self.port
    }

    /// The proxy URL for HTTP clients, e.g. `http://proxy.example:8080`.
    pub fn url(&self) -> String {
        if self.host().contains(':') {
            format!("http://[{}]:{}", self.host(), self.port)
        } else {
            format!("http://{}:{}", self.host(), self.port)
        }
    }
}

impl std::fmt::Debug for ManualProxy {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("ManualProxy")
            .field("host", &self.host())
            .field("port", &self.port)
            .finish()
    }
}

/// Bounded, provider-level network configuration shared by control and archive
/// construction. It carries at most one credential-free manual proxy authority;
/// credentials and provider host authority remain owned by provider clients.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ProviderNetworkPolicy {
    route: ProxyPolicy,
}

impl ProviderNetworkPolicy {
    pub const fn system() -> Self {
        Self {
            route: ProxyPolicy::System,
        }
    }

    pub const fn direct() -> Self {
        Self {
            route: ProxyPolicy::Direct,
        }
    }

    pub const fn manual(proxy: ManualProxy) -> Self {
        Self {
            route: ProxyPolicy::Manual(proxy),
        }
    }

    pub const fn route(self) -> ProxyPolicy {
        self.route
    }
}

pub fn normalize_dns_name(value: &str) -> Result<String, &'static str> {
    let lowered = value.trim().to_ascii_lowercase();
    let normalized = lowered.strip_suffix('.').unwrap_or(&lowered);
    let labels_are_valid = normalized.split('.').all(|label| {
        !label.is_empty()
            && label.len() <= 63
            && !label.starts_with('-')
            && !label.ends_with('-')
            && label
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-')
    });
    if normalized.is_empty()
        || normalized.len() > 253
        || normalized.ends_with('.')
        || !normalized.is_ascii()
        || !labels_are_valid
    {
        return Err("invalid_dns_name");
    }
    Ok(normalized.to_string())
}

pub fn dns_name_matches(host: &str, allowed: &str) -> bool {
    let Ok(host) = normalize_dns_name(host) else {
        return false;
    };
    let Ok(allowed) = normalize_dns_name(allowed) else {
        return false;
    };
    host == allowed
        || host
            .strip_suffix(&allowed)
            .is_some_and(|prefix| prefix.ends_with('.'))
}
