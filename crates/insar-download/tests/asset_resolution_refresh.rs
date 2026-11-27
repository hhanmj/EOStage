use std::fs;
use std::path::PathBuf;
use std::time::{Duration, SystemTime};

use insar_core::StableAssetReference;
use insar_download::{
    evaluate_refresh, AuthRefreshBudget, Clock, RefreshDecision, RefreshEvaluation, RefreshReason,
    RemoteValidator, ResolvedAsset, ResolvedChecksum, ResolvedMethod,
};

struct FixedClock(SystemTime);

impl Clock for FixedClock {
    fn now(&self) -> SystemTime {
        self.0
    }
}

fn reference(asset_key: &str) -> StableAssetReference {
    StableAssetReference {
        provider_id: "ASF".to_string(),
        collection_id: "SENTINEL-1".to_string(),
        provider_product_id: "S1A_FIXTURE".to_string(),
        provider_product_key: "ASF::SENTINEL-1::S1A_FIXTURE".to_string(),
        canonical_product_key: None,
        asset_key: asset_key.to_string(),
        stable_asset_key: format!("ASF::SENTINEL-1::S1A_FIXTURE::{asset_key}"),
    }
}

fn resolved(
    url: &str,
    size: u64,
    checksum: &str,
    etag: &str,
    last_modified: &str,
) -> ResolvedAsset {
    ResolvedAsset::new(url, ResolvedMethod::Get)
        .unwrap()
        .with_expiry(SystemTime::UNIX_EPOCH + Duration::from_secs(200))
        .with_expected_size(Some(size))
        .with_checksum(Some(ResolvedChecksum::new("sha256", checksum).unwrap()))
        .with_remote_validator(RemoteValidator {
            etag: Some(etag.to_string()),
            last_modified: Some(last_modified.to_string()),
            content_length: Some(size),
        })
}

#[test]
fn one_unauthorized_refresh_can_resume_the_same_stable_asset() {
    let old = resolved(
        "https://example.org/file.bin?old=REDACTED",
        4096,
        "same-checksum",
        "same-etag",
        "Wed, 01 Jan 2025 00:00:00 GMT",
    );
    let fresh = resolved(
        "https://example.org/file.bin?new=REDACTED",
        4096,
        "same-checksum",
        "same-etag",
        "Wed, 01 Jan 2025 00:00:00 GMT",
    );
    let identity = reference("primary");
    let mut budget = AuthRefreshBudget::default();
    let permit = budget.try_consume().unwrap();
    let decision = evaluate_refresh(RefreshEvaluation {
        reason: RefreshReason::Unauthorized,
        permit,
        previous_reference: &identity,
        refreshed_reference: &identity,
        previous: &old,
        refreshed: &fresh,
        completed_bytes: 1024,
        prior_response_validator: None,
        clock: &FixedClock(SystemTime::UNIX_EPOCH + Duration::from_secs(100)),
    })
    .unwrap();
    assert_eq!(decision, RefreshDecision::ResumeFrom { offset: 1024 });
    assert_eq!(budget.remaining(), 0);

    assert_eq!(
        budget.try_consume().unwrap_err().code(),
        "ASSET_REFRESH_BUDGET_EXHAUSTED"
    );
}

#[test]
fn stable_reference_mismatch_is_never_resumed() {
    let old = resolved("https://example.org/a.bin", 4096, "same", "same", "same");
    let fresh = resolved("https://example.org/b.bin", 4096, "same", "same", "same");
    let mut budget = AuthRefreshBudget::default();
    let permit = budget.try_consume().unwrap();
    assert_eq!(
        evaluate_refresh(RefreshEvaluation {
            reason: RefreshReason::Expired,
            permit,
            previous_reference: &reference("primary"),
            refreshed_reference: &reference("metadata"),
            previous: &old,
            refreshed: &fresh,
            completed_bytes: 512,
            prior_response_validator: None,
            clock: &FixedClock(SystemTime::UNIX_EPOCH),
        })
        .unwrap_err()
        .code(),
        "ASSET_REFERENCE_INVALID"
    );
}

#[test]
fn size_checksum_etag_and_last_modified_conflicts_are_rejected() {
    let identity = reference("primary");
    let clock = FixedClock(SystemTime::UNIX_EPOCH);
    let inconsistent_content_length =
        resolved("https://example.org/new.bin", 4096, "sum", "tag", "date").with_remote_validator(
            RemoteValidator {
                etag: Some("tag".to_string()),
                last_modified: Some("date".to_string()),
                content_length: Some(8192),
            },
        );
    let cases = [
        (
            resolved("https://example.org/old.bin", 4096, "sum", "tag", "date"),
            resolved("https://example.org/new.bin", 8192, "sum", "tag", "date"),
        ),
        (
            resolved("https://example.org/old.bin", 4096, "old", "tag", "date"),
            resolved("https://example.org/new.bin", 4096, "new", "tag", "date"),
        ),
        (
            resolved("https://example.org/old.bin", 4096, "sum", "old", "date"),
            resolved("https://example.org/new.bin", 4096, "sum", "new", "date"),
        ),
        (
            resolved("https://example.org/old.bin", 4096, "sum", "tag", "old"),
            resolved("https://example.org/new.bin", 4096, "sum", "tag", "new"),
        ),
        (
            resolved("https://example.org/old.bin", 4096, "sum", "tag", "date"),
            inconsistent_content_length,
        ),
    ];

    for (old, fresh) in cases {
        let mut budget = AuthRefreshBudget::default();
        let permit = budget.try_consume().unwrap();
        assert_eq!(
            evaluate_refresh(RefreshEvaluation {
                reason: RefreshReason::Unauthorized,
                permit,
                previous_reference: &identity,
                refreshed_reference: &identity,
                previous: &old,
                refreshed: &fresh,
                completed_bytes: 512,
                prior_response_validator: None,
                clock: &clock,
            })
            .unwrap_err()
            .code(),
            "ASSET_RESOLUTION_FAILED"
        );
    }
}

#[test]
fn missing_content_identity_restarts_instead_of_unsafe_range_resume() {
    let old = ResolvedAsset::new("https://example.org/a", ResolvedMethod::Get)
        .unwrap()
        .with_expected_size(Some(4096));
    let fresh = ResolvedAsset::new("https://example.org/b", ResolvedMethod::Get)
        .unwrap()
        .with_expected_size(Some(4096));
    let identity = reference("primary");
    let mut budget = AuthRefreshBudget::default();
    let permit = budget.try_consume().unwrap();

    assert_eq!(
        evaluate_refresh(RefreshEvaluation {
            reason: RefreshReason::Unauthorized,
            permit,
            previous_reference: &identity,
            refreshed_reference: &identity,
            previous: &old,
            refreshed: &fresh,
            completed_bytes: 512,
            prior_response_validator: None,
            clock: &FixedClock(SystemTime::UNIX_EPOCH),
        })
        .unwrap(),
        RefreshDecision::RestartFromBeginning
    );
}

#[test]
fn zero_downloaded_bytes_restart_without_claiming_range_resume() {
    let old = resolved("https://example.org/a", 10, "sum", "tag", "date");
    let fresh = resolved("https://example.org/b", 10, "sum", "tag", "date");
    let identity = reference("primary");
    let mut budget = AuthRefreshBudget::default();
    let permit = budget.try_consume().unwrap();
    assert_eq!(
        evaluate_refresh(RefreshEvaluation {
            reason: RefreshReason::Expired,
            permit,
            previous_reference: &identity,
            refreshed_reference: &identity,
            previous: &old,
            refreshed: &fresh,
            completed_bytes: 0,
            prior_response_validator: None,
            clock: &FixedClock(SystemTime::UNIX_EPOCH),
        })
        .unwrap(),
        RefreshDecision::RestartFromBeginning
    );
}

#[test]
fn refresh_module_is_pure_and_cannot_touch_transfer_or_part_files() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    let source =
        fs::read_to_string(root.join("crates/insar-download/src/asset_resolver/refresh.rs"))
            .unwrap();
    for forbidden in [
        "HttpTransfer",
        "ProductionDownloadWorker",
        ".part",
        "std::fs",
    ] {
        assert!(
            !source.contains(forbidden),
            "refresh coordination crossed production boundary: {forbidden}"
        );
    }
}

#[test]
fn prior_runtime_validator_controls_refresh_resume_without_checksum() {
    let old = ResolvedAsset::new("https://example.org/old", ResolvedMethod::Get)
        .unwrap()
        .with_expected_size(Some(4096));
    let fresh = ResolvedAsset::new("https://example.org/fresh", ResolvedMethod::Get)
        .unwrap()
        .with_expected_size(Some(4096));
    let identity = reference("primary");
    let clock = FixedClock(SystemTime::UNIX_EPOCH);

    for prior in [
        RemoteValidator {
            etag: Some("\"strong\"".to_string()),
            last_modified: None,
            content_length: Some(4096),
        },
        RemoteValidator {
            etag: None,
            last_modified: Some("Wed, 01 Jan 2025 00:00:00 GMT".to_string()),
            content_length: Some(4096),
        },
    ] {
        let mut budget = AuthRefreshBudget::default();
        let permit = budget.try_consume().unwrap();
        assert_eq!(
            evaluate_refresh(RefreshEvaluation {
                reason: RefreshReason::Unauthorized,
                permit,
                previous_reference: &identity,
                refreshed_reference: &identity,
                previous: &old,
                refreshed: &fresh,
                completed_bytes: 512,
                prior_response_validator: Some(&prior),
                clock: &clock,
            })
            .unwrap(),
            RefreshDecision::ResumeFrom { offset: 512 }
        );
    }

    let weak = RemoteValidator {
        etag: Some("W/\"weak\"".to_string()),
        last_modified: None,
        content_length: Some(4096),
    };
    let mut budget = AuthRefreshBudget::default();
    let permit = budget.try_consume().unwrap();
    assert_eq!(
        evaluate_refresh(RefreshEvaluation {
            reason: RefreshReason::Unauthorized,
            permit,
            previous_reference: &identity,
            refreshed_reference: &identity,
            previous: &old,
            refreshed: &fresh,
            completed_bytes: 512,
            prior_response_validator: Some(&weak),
            clock: &clock,
        })
        .unwrap(),
        RefreshDecision::RestartFromBeginning
    );
}

#[test]
fn prior_runtime_validator_conflict_is_terminal() {
    let old = ResolvedAsset::new("https://example.org/old", ResolvedMethod::Get)
        .unwrap()
        .with_expected_size(Some(4096));
    let fresh = ResolvedAsset::new("https://example.org/fresh", ResolvedMethod::Get)
        .unwrap()
        .with_expected_size(Some(4096))
        .with_remote_validator(RemoteValidator {
            etag: Some("\"new\"".to_string()),
            last_modified: None,
            content_length: Some(4096),
        });
    let prior = RemoteValidator {
        etag: Some("\"old\"".to_string()),
        last_modified: None,
        content_length: Some(4096),
    };
    let identity = reference("primary");
    let mut budget = AuthRefreshBudget::default();
    let permit = budget.try_consume().unwrap();
    assert_eq!(
        evaluate_refresh(RefreshEvaluation {
            reason: RefreshReason::Unauthorized,
            permit,
            previous_reference: &identity,
            refreshed_reference: &identity,
            previous: &old,
            refreshed: &fresh,
            completed_bytes: 512,
            prior_response_validator: Some(&prior),
            clock: &FixedClock(SystemTime::UNIX_EPOCH),
        })
        .unwrap_err()
        .code(),
        "ASSET_RESOLUTION_FAILED"
    );
}
