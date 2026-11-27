use super::*;

#[test]
fn equivalent_utc_fraction_spellings_preserve_the_exact_instant() {
    for (raw, expected) in [
        ("2025-01-29T02:28:49.024000Z", "2025-01-29T02:28:49.024Z"),
        ("2025-01-29T02:28:49.000000Z", "2025-01-29T02:28:49Z"),
        (
            "2025-01-29T02:28:49.123456789Z",
            "2025-01-29T02:28:49.123456789Z",
        ),
        (
            "2025-01-29T02:28:49.024000+00:00",
            "2025-01-29T02:28:49.024Z",
        ),
    ] {
        let normalized = normalized_utc(raw).unwrap();
        assert_eq!(normalized, expected);
        assert_eq!(
            DateTime::parse_from_rfc3339(raw).unwrap(),
            DateTime::parse_from_rfc3339(&normalized).unwrap()
        );
    }
}

#[test]
fn normalization_must_not_silently_truncate_precision_or_infer_a_timezone() {
    for raw in [
        "2025-01-29T02:28:49.1234567891Z",
        "2025-01-29T02:28:49.1234567890Z",
        "2025-01-29T02:28:49.024000+08:00",
        "2025-01-29T02:28:49.024000-00:00",
        "2025-01-29T02:28:49.024000",
    ] {
        assert!(normalized_utc(raw).is_err(), "unexpectedly accepted {raw}");
    }
}
