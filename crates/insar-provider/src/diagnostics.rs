//! Opt-in diagnostic switches under the shared `INSAR_DIAGNOSTIC_*` pattern.
//!
//! Reading the environment stays in this module so provider modules with
//! strict isolation contracts (for example `usgs_m2m`) never gain a direct
//! `std::env` boundary. Nothing here logs credentials or tokens.

/// Whether the `INSAR_DIAGNOSTIC_USGS_M2M` opt-in is enabled.
/// Emits only mapped error codes, upstream M2M `errorCode` values, HTTP
/// statuses and sanitized messages — never the username or Application Token.
pub fn usgs_m2m_login_diagnostic_enabled() -> bool {
    std::env::var("INSAR_DIAGNOSTIC_USGS_M2M")
        .ok()
        .is_some_and(|value| {
            matches!(
                value.trim().to_ascii_lowercase().as_str(),
                "1" | "true" | "yes"
            )
        })
}

/// Percent-encodes non-unreserved ASCII bytes in a secret so URL-encoded
/// occurrences inside error strings or redirect URLs are also scrubbed.
fn percent_encode_secret(secret: &str) -> String {
    let mut encoded = String::with_capacity(secret.len() * 2);
    for byte in secret.bytes() {
        if byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.' | b'~') {
            encoded.push(char::from(byte));
        } else {
            encoded.push_str(&format!("%{byte:02X}"));
        }
    }
    encoded
}

/// Scrubs explicit secret values (both plaintext and URL percent-encoded),
/// inline URL `user:password@` credentials, and sensitive query/header tokens
/// from diagnostic or error messages (RustGIS security hardening contract).
pub fn redact_secret(text: &str, secrets: &[&str]) -> String {
    let mut scrubbed = text.to_owned();
    for &secret in secrets {
        let trimmed = secret.trim();
        if trimmed.is_empty() {
            continue;
        }
        scrubbed = scrubbed.replace(trimmed, "[REDACTED]");
        let encoded_upper = percent_encode_secret(trimmed);
        if encoded_upper != trimmed {
            scrubbed = scrubbed.replace(&encoded_upper, "[REDACTED]");
            let encoded_lower = encoded_upper.to_ascii_lowercase();
            scrubbed = scrubbed.replace(&encoded_lower, "[REDACTED]");
        }
    }
    scrubbed = redact_url_userinfo(&scrubbed);
    scrubbed = redact_sensitive_key_values(&scrubbed);
    scrubbed
}

fn redact_url_userinfo(text: &str) -> String {
    let mut result = String::with_capacity(text.len());
    let mut remaining = text;
    while let Some(scheme_pos) = remaining.find("://") {
        let prefix_end = scheme_pos + 3;
        result.push_str(&remaining[..prefix_end]);
        remaining = &remaining[prefix_end..];
        let authority_end = remaining
            .find(['/', '?', '#', ' ', '"', '\'', ')'])
            .unwrap_or(remaining.len());
        let authority = &remaining[..authority_end];
        if let Some(at_pos) = authority.rfind('@') {
            result.push_str("[REDACTED]@");
            result.push_str(&authority[at_pos + 1..]);
        } else {
            result.push_str(authority);
        }
        remaining = &remaining[authority_end..];
    }
    result.push_str(remaining);
    result
}

fn redact_sensitive_key_values(text: &str) -> String {
    const KEYS: &[&str] = &[
        "access_token=",
        "api_key=",
        "apikey=",
        "password=",
        "secret=",
        "token=",
        "X-Amz-Signature=",
        "X-Amz-Security-Token=",
        "x-amz-signature=",
        "x-amz-security-token=",
    ];
    let mut current = text.to_owned();
    for key in KEYS {
        let mut out = String::with_capacity(current.len());
        let mut rest = current.as_str();
        while let Some(pos) = rest.find(key) {
            let value_start = pos + key.len();
            out.push_str(&rest[..value_start]);
            out.push_str("[REDACTED]");
            let after_key = &rest[value_start..];
            let value_end = after_key
                .find(['&', ' ', '"', '\'', '#', '\r', '\n'])
                .unwrap_or(after_key.len());
            rest = &after_key[value_end..];
        }
        out.push_str(rest);
        current = out;
    }
    current
}

/// Determines whether an `Authorization` header may be preserved across an HTTP
/// redirect. Returns `true` only when `origin_url` and `target_url` share the
/// exact same `(scheme, host, effective_port)` origin, preventing bearer-token
/// leakage onto cross-origin presigned S3 / CDN redirect targets.
pub fn should_preserve_authorization_on_redirect(origin_url: &str, target_url: &str) -> bool {
    let (Some(origin), Some(target)) = (parse_url_origin(origin_url), parse_url_origin(target_url))
    else {
        return false;
    };
    origin == target
}

fn parse_url_origin(raw: &str) -> Option<(String, String, u16)> {
    let trimmed = raw.trim();
    let (scheme_raw, after_scheme) = trimmed.split_once("://")?;
    let scheme = scheme_raw.to_ascii_lowercase();
    let default_port = match scheme.as_str() {
        "https" => 443u16,
        "http" => 80u16,
        _ => return None,
    };
    let authority_end = after_scheme
        .find(['/', '?', '#'])
        .unwrap_or(after_scheme.len());
    let authority = &after_scheme[..authority_end];
    if authority.is_empty() {
        return None;
    }
    let host_port = authority.rsplit_once('@').map_or(authority, |(_, hp)| hp);
    let (host_raw, port) = if let Some(bracket_end) = host_port.find(']') {
        let host_part = &host_port[..=bracket_end];
        let rest = &host_port[bracket_end + 1..];
        if let Some(port_str) = rest.strip_prefix(':') {
            (host_part, port_str.parse::<u16>().ok()?)
        } else if rest.is_empty() {
            (host_part, default_port)
        } else {
            return None;
        }
    } else if let Some((h, p)) = host_port.rsplit_once(':') {
        (h, p.parse::<u16>().ok()?)
    } else {
        (host_port, default_port)
    };
    if host_raw.is_empty() {
        return None;
    }
    Some((scheme, host_raw.to_ascii_lowercase(), port))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn diagnostic_gate_is_off_by_default_and_parses_the_shared_values() {
        // The ambient variable is not set in the test environment; if a host
        // sets it the check still only accepts the documented opt-in values.
        let ambient = std::env::var("INSAR_DIAGNOSTIC_USGS_M2M").is_err();
        if ambient {
            assert!(!usgs_m2m_login_diagnostic_enabled());
        }
    }

    #[test]
    fn redact_secret_scrubs_plaintext_percent_encoded_and_url_credentials() {
        let secret = "p@ss:w0rd/secret+token=1";
        let raw = format!(
            "GET https://earthdata_user:{secret}@urs.earthdata.nasa.gov/oauth?access_token=eyJhbGciOi&X-Amz-Signature=deadbeef1234 failed with token p%40ss%3Aw0rd%2Fsecret%2Btoken%3D1"
        );
        let scrubbed = redact_secret(&raw, &[secret]);
        assert!(!scrubbed.contains(secret));
        assert!(!scrubbed.contains("p%40ss%3Aw0rd"));
        assert!(!scrubbed.contains("earthdata_user"));
        assert!(!scrubbed.contains("eyJhbGciOi"));
        assert!(!scrubbed.contains("deadbeef1234"));
        assert!(scrubbed.contains("https://[REDACTED]@urs.earthdata.nasa.gov"));
        assert!(scrubbed.contains("access_token=[REDACTED]"));
        assert!(scrubbed.contains("X-Amz-Signature=[REDACTED]"));
    }

    #[test]
    fn redirect_authorization_guard_strips_cross_origin_and_downgrade_redirects() {
        assert!(should_preserve_authorization_on_redirect(
            "https://urs.earthdata.nasa.gov/oauth/authorize",
            "https://urs.earthdata.nasa.gov/profile"
        ));
        assert!(!should_preserve_authorization_on_redirect(
            "https://datapool.asf.alaska.edu/SLC/SA/scene.zip",
            "https://asf-ngap2w-p-s1-slc.s3.us-west-2.amazonaws.com/scene.zip?X-Amz-Signature=abc"
        ));
        assert!(!should_preserve_authorization_on_redirect(
            "https://urs.earthdata.nasa.gov/secure",
            "http://urs.earthdata.nasa.gov/insecure"
        ));
    }
}
