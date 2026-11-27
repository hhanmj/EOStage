//! Sanitized facts about a rejected (401/403) Bhoonidhi download probe.
//!
//! The probe body and headers can carry tokens or account details, so only
//! fixed labels and numbers leave this module: the `WWW-Authenticate` class,
//! the content-type class, the first matching keyword class of the bounded
//! body, timings, the body length and the server clock skew.

use std::collections::VecDeque;
use std::sync::Mutex;
use std::time::{SystemTime, UNIX_EPOCH};

use reqwest::header::{HeaderMap, CONTENT_TYPE, DATE, WWW_AUTHENTICATE};

const RECENT_REJECTION_LIMIT: usize = 8;
const KEYWORD_CLASSES: &[(&str, &[&str])] = &[
    ("expired", &["expired", "expiry"]),
    ("invalid", &["invalid"]),
    ("session", &["session"]),
    ("quota", &["quota", "limit", "too many"]),
    (
        "permission",
        &[
            "permission",
            "forbidden",
            "not authorized",
            "unauthorized",
            "access denied",
        ],
    ),
    ("license", &["license", "licence", "subscription"]),
];

static RECENT_REJECTIONS: Mutex<VecDeque<ProbeRejectionFacts>> = Mutex::new(VecDeque::new());

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct ProbeRejectionFacts {
    pub(crate) status: u16,
    pub(crate) www_authenticate: &'static str,
    pub(crate) content_type: &'static str,
    pub(crate) body_keyword: &'static str,
    pub(crate) headers_ms: u64,
    pub(crate) body_ms: u64,
    pub(crate) body_len: usize,
    pub(crate) clock_skew_seconds: Option<i64>,
}

impl ProbeRejectionFacts {
    pub(crate) fn line(&self) -> String {
        let skew = self
            .clock_skew_seconds
            .map_or_else(|| "unknown".to_owned(), |value| value.to_string());
        format!(
            "[bhoonidhi-probe-rejection] status={} wa={} ct={} kw={} hdr_ms={} body_ms={} len={} skew_s={}",
            self.status,
            self.www_authenticate,
            self.content_type,
            self.body_keyword,
            self.headers_ms,
            self.body_ms,
            self.body_len,
            skew
        )
    }
}

/// Facts for a 401/403 probe response; `None` for every other status.
pub(crate) fn rejection_facts(
    status: u16,
    headers: &HeaderMap,
    body_prefix: &[u8],
    body_len: usize,
    headers_ms: u64,
    body_ms: u64,
    now: SystemTime,
) -> Option<ProbeRejectionFacts> {
    if !matches!(status, 401 | 403) {
        return None;
    }
    let header = |name| headers.get(name).and_then(|value| value.to_str().ok());
    Some(ProbeRejectionFacts {
        status,
        www_authenticate: www_authenticate_class(header(WWW_AUTHENTICATE)),
        content_type: content_type_class(header(CONTENT_TYPE)),
        body_keyword: body_keyword_class(body_prefix),
        headers_ms,
        body_ms,
        body_len,
        clock_skew_seconds: header(DATE).and_then(|date| clock_skew_seconds(date, now)),
    })
}

/// Keeps the facts for later inspection and writes one sanitized log line.
pub(crate) fn record_probe_rejection(facts: ProbeRejectionFacts) {
    eprintln!("{}", facts.line());
    if let Ok(mut recent) = RECENT_REJECTIONS.lock() {
        if recent.len() == RECENT_REJECTION_LIMIT {
            recent.pop_front();
        }
        recent.push_back(facts);
    }
}

#[cfg(test)]
pub(crate) fn recent_probe_rejections() -> Vec<ProbeRejectionFacts> {
    RECENT_REJECTIONS
        .lock()
        .map(|recent| recent.iter().cloned().collect())
        .unwrap_or_default()
}

fn www_authenticate_class(value: Option<&str>) -> &'static str {
    let Some(value) = value else {
        return "none";
    };
    let value = value.to_ascii_lowercase();
    if value.contains("invalid_token") {
        "invalid_token"
    } else if value.contains("insufficient_scope") {
        "insufficient_scope"
    } else if value.trim_start().starts_with("bearer") {
        "bearer"
    } else {
        "other"
    }
}

fn content_type_class(value: Option<&str>) -> &'static str {
    let Some(value) = value else {
        return "none";
    };
    let value = value.to_ascii_lowercase();
    if value.contains("json") {
        "json"
    } else if value.contains("html") {
        "html"
    } else if value.starts_with("text/") {
        "text"
    } else {
        "other"
    }
}

fn body_keyword_class(body_prefix: &[u8]) -> &'static str {
    let text = String::from_utf8_lossy(body_prefix).to_ascii_lowercase();
    KEYWORD_CLASSES
        .iter()
        .find(|(_, needles)| needles.iter().any(|needle| text.contains(needle)))
        .map_or("none", |(class, _)| *class)
}

/// Server `Date` minus local time, for an RFC 9110 IMF-fixdate such as
/// `Sun, 06 Nov 1994 08:49:37 GMT`.
fn clock_skew_seconds(date: &str, now: SystemTime) -> Option<i64> {
    let server = imf_fixdate_unix_seconds(date)?;
    let local = i64::try_from(now.duration_since(UNIX_EPOCH).ok()?.as_secs()).ok()?;
    server.checked_sub(local)
}

fn imf_fixdate_unix_seconds(date: &str) -> Option<i64> {
    let (_, rest) = date.trim().split_once(", ")?;
    let mut parts = rest.split(' ');
    let day: i64 = parts.next()?.parse().ok()?;
    let month = match parts.next()? {
        "Jan" => 1,
        "Feb" => 2,
        "Mar" => 3,
        "Apr" => 4,
        "May" => 5,
        "Jun" => 6,
        "Jul" => 7,
        "Aug" => 8,
        "Sep" => 9,
        "Oct" => 10,
        "Nov" => 11,
        "Dec" => 12,
        _ => return None,
    };
    let year: i64 = parts.next()?.parse().ok()?;
    let mut clock = parts.next()?.split(':');
    let hour: i64 = clock.next()?.parse().ok()?;
    let minute: i64 = clock.next()?.parse().ok()?;
    let second: i64 = clock.next()?.parse().ok()?;
    if parts.next()? != "GMT"
        || parts.next().is_some()
        || clock.next().is_some()
        || !(1..=31).contains(&day)
        || hour > 23
        || minute > 59
        || second > 60
    {
        return None;
    }
    // Days from 1970-01-01 to the civil date (Howard Hinnant's algorithm).
    let y = if month <= 2 { year - 1 } else { year };
    let era = y.div_euclid(400);
    let year_of_era = y - era * 400;
    let shifted_month = (month + 9) % 12;
    let day_of_year = (153 * shifted_month + 2) / 5 + day - 1;
    let day_of_era = year_of_era * 365 + year_of_era / 4 - year_of_era / 100 + day_of_year;
    let days = era * 146_097 + day_of_era - 719_468;
    Some(days * 86_400 + hour * 3_600 + minute * 60 + second)
}

#[cfg(test)]
mod tests {
    use super::*;
    use reqwest::header::HeaderValue;
    use std::time::Duration;

    fn headers(pairs: &[(reqwest::header::HeaderName, &str)]) -> HeaderMap {
        let mut map = HeaderMap::new();
        for (name, value) in pairs {
            map.insert(name.clone(), HeaderValue::from_str(value).unwrap());
        }
        map
    }

    #[test]
    fn a_401_probe_keeps_only_classes_and_numbers() {
        let now = UNIX_EPOCH + Duration::from_secs(784_111_777); // Sun, 06 Nov 1994 08:49:37 GMT
        let body = br#"{"error":"token expired","token":"SECRET-ACCESS-TOKEN-VALUE"}"#;
        let facts = rejection_facts(
            401,
            &headers(&[
                (
                    WWW_AUTHENTICATE,
                    r#"Bearer error="invalid_token", error_description="SECRET-DESCRIPTION""#,
                ),
                (CONTENT_TYPE, "application/json; charset=utf-8"),
                (DATE, "Sun, 06 Nov 1994 09:14:37 GMT"),
            ]),
            body,
            body.len(),
            29_850,
            3,
            now,
        )
        .expect("401 is a rejection");
        assert_eq!(
            facts,
            ProbeRejectionFacts {
                status: 401,
                www_authenticate: "invalid_token",
                content_type: "json",
                body_keyword: "expired",
                headers_ms: 29_850,
                body_ms: 3,
                body_len: body.len(),
                clock_skew_seconds: Some(1_500),
            }
        );
        let line = facts.line();
        assert_eq!(
            line,
            format!(
                "[bhoonidhi-probe-rejection] status=401 wa=invalid_token ct=json kw=expired hdr_ms=29850 body_ms=3 len={} skew_s=1500",
                body.len()
            )
        );
        assert!(!line.contains("SECRET"));
    }

    #[test]
    fn only_401_and_403_probes_are_rejections() {
        let now = SystemTime::now();
        assert!(rejection_facts(206, &HeaderMap::new(), b"", 0, 1, 0, now).is_none());
        assert!(rejection_facts(500, &HeaderMap::new(), b"", 0, 1, 0, now).is_none());
        let forbidden = rejection_facts(
            403,
            &HeaderMap::new(),
            b"<html>Access Denied</html>",
            26,
            5,
            1,
            now,
        )
        .expect("403 is a rejection");
        assert_eq!(forbidden.www_authenticate, "none");
        assert_eq!(forbidden.content_type, "none");
        assert_eq!(forbidden.body_keyword, "permission");
        assert_eq!(forbidden.clock_skew_seconds, None);
    }

    #[test]
    fn header_and_body_classes_are_fixed_labels() {
        assert_eq!(www_authenticate_class(Some("Bearer realm=\"x\"")), "bearer");
        assert_eq!(
            www_authenticate_class(Some("Bearer error=\"insufficient_scope\"")),
            "insufficient_scope"
        );
        assert_eq!(www_authenticate_class(Some("Basic realm=\"x\"")), "other");
        assert_eq!(content_type_class(Some("text/html")), "html");
        assert_eq!(content_type_class(Some("text/plain")), "text");
        assert_eq!(
            content_type_class(Some("application/octet-stream")),
            "other"
        );
        assert_eq!(body_keyword_class(b"Daily download quota reached"), "quota");
        assert_eq!(body_keyword_class(b"Session not found"), "session");
        assert_eq!(body_keyword_class(b"License required"), "license");
        assert_eq!(body_keyword_class(b"\xff\xfe binary"), "none");
    }

    #[test]
    fn server_dates_parse_only_in_the_http_format() {
        assert_eq!(
            imf_fixdate_unix_seconds("Sun, 06 Nov 1994 08:49:37 GMT"),
            Some(784_111_777)
        );
        assert_eq!(
            imf_fixdate_unix_seconds("Thu, 01 Jan 1970 00:00:00 GMT"),
            Some(0)
        );
        assert_eq!(
            imf_fixdate_unix_seconds("Wed, 21 Oct 2026 16:34:20 GMT"),
            Some(1_792_600_460)
        );
        assert_eq!(
            imf_fixdate_unix_seconds("Sunday, 06-Nov-94 08:49:37 GMT"),
            None
        );
        assert_eq!(
            imf_fixdate_unix_seconds("Sun, 06 Nov 1994 08:49:37 UTC"),
            None
        );
        assert_eq!(imf_fixdate_unix_seconds("garbage"), None);
    }

    #[test]
    fn recorded_rejections_keep_the_latest_eight() {
        let facts = |status| ProbeRejectionFacts {
            status,
            www_authenticate: "none",
            content_type: "none",
            body_keyword: "none",
            headers_ms: 0,
            body_ms: 0,
            body_len: 0,
            clock_skew_seconds: None,
        };
        for index in 0..10 {
            record_probe_rejection(facts(if index % 2 == 0 { 401 } else { 403 }));
        }
        assert_eq!(recent_probe_rejections().len(), RECENT_REJECTION_LIMIT);
    }
}
