//! The bounded automatic retry of one transient network request. The NISAR
//! science Ranges and planning preflight (design 2026-10-17 §3) and the
//! Bhoonidhi download probe (downloader enhancement D1d) share its attempt
//! limit and backoff, and wait in a way a pause or cancel ends at once.

use std::time::{Duration, Instant};

/// The first attempt plus two automatic retries.
pub(crate) const TRANSIENT_MAX_ATTEMPTS: u32 = 3;

/// Backoff before automatic retry `retry` (1-based): 5 s, then 20 s; an HTTP
/// 429 waits at least 30 s.
pub(crate) fn transient_retry_backoff(retry: u32, http_status: Option<u16>) -> Duration {
    let seconds = if retry <= 1 { 5 } else { 20 };
    let seconds = if http_status == Some(429) {
        seconds.max(30)
    } else {
        seconds
    };
    Duration::from_secs(seconds)
}

/// Parses an HTTP `Retry-After` delta-seconds header value, clamped safely to `[0, 120]s`.
pub(crate) fn parse_retry_after_header(value: &str) -> Option<Duration> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return None;
    }
    trimmed
        .parse::<u64>()
        .ok()
        .map(|secs| Duration::from_secs(secs.min(120)))
}

/// Computes rate-limit-aware retry delay: preserves the baseline
/// `transient_retry_backoff`, respects an upstream `Retry-After` header up to 120 s,
/// and applies bounded sub-second/multi-second jitter (`<= 2500 ms`).
#[allow(dead_code)]
pub(crate) fn paced_retry_delay(
    retry: u32,
    http_status: Option<u16>,
    retry_after: Option<Duration>,
    jitter_ms: u64,
) -> Duration {
    let base = transient_retry_backoff(retry, http_status);
    let respected = match retry_after {
        Some(hint) => base.max(hint.min(Duration::from_secs(120))),
        None => base,
    };
    respected.saturating_add(Duration::from_millis(jitter_ms.min(2_500)))
}

/// Classifies a reqwest error source chain into fine-grained diagnostics (`timeout`, `dns`,
/// `tls`, `proxy`, `stream_interrupted`, `connect`, or `unknown`).
pub(crate) fn classify_reqwest_network_cause(error: &reqwest::Error) -> &'static str {
    let mut chain_text = error.to_string();
    let mut cur: Option<&(dyn std::error::Error + 'static)> = std::error::Error::source(error);
    while let Some(src) = cur {
        chain_text.push(' ');
        chain_text.push_str(&src.to_string());
        cur = src.source();
    }
    let text_class = classify_network_error_text(&chain_text);
    if text_class != "unknown" {
        return text_class;
    }
    if error.is_timeout() {
        "timeout"
    } else if error.is_body() || error.is_decode() {
        "stream_interrupted"
    } else if error.is_connect() {
        "connect"
    } else {
        "unknown"
    }
}

pub(crate) fn classify_network_error_text(raw: &str) -> &'static str {
    let lower = raw.to_ascii_lowercase();
    if lower.contains("invalid peer certificate")
        || lower.contains("certificate")
        || lower.contains("unknownissuer")
        || lower.contains("tls")
        || lower.contains("ssl")
        || lower.contains("handshake")
    {
        return "tls";
    }
    if lower.contains("proxy") || lower.contains("socks") {
        return "proxy";
    }
    if lower.contains("dns")
        || lower.contains("failed to lookup address")
        || lower.contains("nodename nor servname")
        || lower.contains("name or service not known")
        || lower.contains("no such host")
        || lower.contains("getaddrinfo")
    {
        return "dns";
    }
    if lower.contains("timed out") || lower.contains("timeout") || lower.contains("deadline") {
        return "timeout";
    }
    if lower.contains("unexpected eof")
        || lower.contains("early eof")
        || lower.contains("connection reset")
        || lower.contains("broken pipe")
        || lower.contains("stream")
        || lower.contains("body error")
    {
        return "stream_interrupted";
    }
    if lower.contains("connection refused")
        || lower.contains("network unreachable")
        || lower.contains("host is unreachable")
        || lower.contains("tcp connect")
        || lower.contains("connect")
    {
        return "connect";
    }
    "unknown"
}

/// Waits `delay` unless `stop` becomes true (polled every 20 ms); returns
/// whether the wait was stopped.
pub(crate) fn wait_unless_stopped(delay: Duration, stop: &dyn Fn() -> bool) -> bool {
    let deadline = Instant::now() + delay;
    loop {
        if stop() {
            return true;
        }
        let now = Instant::now();
        if now >= deadline {
            return false;
        }
        std::thread::sleep((deadline - now).min(Duration::from_millis(20)));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::Arc;

    #[test]
    fn the_backoff_is_five_then_twenty_seconds_and_at_least_thirty_after_429() {
        assert_eq!(transient_retry_backoff(1, None), Duration::from_secs(5));
        assert_eq!(
            transient_retry_backoff(2, Some(503)),
            Duration::from_secs(20)
        );
        assert_eq!(
            transient_retry_backoff(1, Some(429)),
            Duration::from_secs(30)
        );
        assert_eq!(
            transient_retry_backoff(2, Some(429)),
            Duration::from_secs(30)
        );
    }

    #[test]
    fn retry_after_pacing_and_network_cause_diagnostics() {
        assert_eq!(
            parse_retry_after_header("45"),
            Some(Duration::from_secs(45))
        );
        assert_eq!(
            parse_retry_after_header("999"),
            Some(Duration::from_secs(120))
        );
        assert_eq!(parse_retry_after_header(""), None);
        assert_eq!(
            paced_retry_delay(1, Some(429), Some(Duration::from_secs(45)), 350),
            Duration::from_millis(45_350)
        );
        assert_eq!(
            classify_network_error_text("dns error: failed to lookup address information"),
            "dns"
        );
        assert_eq!(
            classify_network_error_text("error trying to connect via HTTP proxy"),
            "proxy"
        );
        assert_eq!(
            classify_network_error_text("unexpected EOF while reading stream"),
            "stream_interrupted"
        );
        assert_eq!(
            classify_network_error_text("invalid peer certificate: UnknownIssuer"),
            "tls"
        );
        assert_eq!(
            classify_network_error_text("operation timed out"),
            "timeout"
        );
    }

    #[test]
    fn a_stop_ends_the_wait_at_once_and_an_elapsed_wait_reports_no_stop() {
        assert!(!wait_unless_stopped(Duration::from_millis(30), &|| false));
        let stop = Arc::new(AtomicBool::new(false));
        let setter = Arc::clone(&stop);
        let thread = std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(60));
            setter.store(true, Ordering::SeqCst);
        });
        let started = Instant::now();
        assert!(wait_unless_stopped(Duration::from_secs(30), &|| stop.load(Ordering::SeqCst)));
        assert!(
            started.elapsed() < Duration::from_secs(5),
            "{:?}",
            started.elapsed()
        );
        thread.join().unwrap();
    }
}
