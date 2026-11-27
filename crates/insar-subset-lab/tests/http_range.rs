mod support;

use std::fs;

use insar_subset_lab::{
    fetch_local_range_plan, ByteRange, LocalRangeSource, MergePolicy, RangeFailureKind, RangePlan,
    RangeTransferLimits, RangeWorkspace,
};
use sha2::{Digest, Sha256};

use support::http::{RangeServer, ResponseMode};
use support::TempRoot;

const ETAG: &str = "\"fixture-v1\"";

#[test]
fn exact_ranges_are_validator_bound_byte_accounted_and_atomically_promoted() {
    let root = TempRoot::new("http-range-green");
    let body = fixture_bytes(2_048, 3);
    let plan = sparse_plan(body.len() as u64);
    let expected = expected_range_set(&body, &plan);
    let server = RangeServer::new(body.clone(), ETAG, ResponseMode::Normal);
    let source = LocalRangeSource::new(server.url(), body.len() as u64, ETAG).unwrap();
    let workspace = RangeWorkspace::new(root.file("range-set.bin")).unwrap();

    let result = fetch_local_range_plan(
        &source,
        &plan,
        &workspace,
        &sha256(&expected),
        &limits(8, 1_024, 17),
        || false,
    )
    .unwrap();

    assert_eq!(fs::read(&result.output_path).unwrap(), expected);
    assert_eq!(result.output_sha256, sha256(&expected));
    assert_eq!(result.strong_etag, ETAG);
    assert!(!result.resumed);
    assert_eq!(result.ledger.request_count, plan.ranges().len() as u64);
    assert_eq!(result.ledger.response_body_bytes, plan.transfer_bytes());
    assert_eq!(result.ledger.accepted_unique_bytes, plan.transfer_bytes());
    assert_eq!(result.ledger.retry_duplicate_bytes, 0);
    assert!(result.workspace_cleanup_complete);
    assert!(result.ledger.planned_transfer_bytes < source.source_len());
    assert!(!workspace.part_path().exists());
    assert!(!workspace.checkpoint_path().exists());
    assert!(!workspace.range_directory().exists());

    let requests = server.requests();
    assert_eq!(requests.len(), plan.ranges().len());
    for (request, range) in requests.iter().zip(plan.ranges()) {
        assert_eq!(request.status, 206);
        assert_eq!(
            request.range.as_deref(),
            Some(format!("bytes={}-{}", range.start(), range.end_exclusive() - 1).as_str())
        );
        assert_eq!(request.if_range.as_deref(), Some(ETAG));
        assert_eq!(request.accept_encoding.as_deref(), Some("identity"));
    }
}

#[test]
fn strict_protocol_faults_never_publish_a_final_output() {
    let cases = [
        (ResponseMode::IgnoreRange, RangeFailureKind::SourceChanged),
        (ResponseMode::BadContentRange, RangeFailureKind::Protocol),
        (ResponseMode::WrongTotal, RangeFailureKind::Protocol),
        (ResponseMode::WrongContentLength, RangeFailureKind::Protocol),
        (ResponseMode::Truncate, RangeFailureKind::Protocol),
        (
            ResponseMode::Corrupt,
            RangeFailureKind::OutputChecksumMismatch,
        ),
        (ResponseMode::MissingEtag, RangeFailureKind::Protocol),
        (ResponseMode::WeakEtag, RangeFailureKind::Protocol),
        (
            ResponseMode::ChangedStrongEtag,
            RangeFailureKind::SourceChanged,
        ),
        (ResponseMode::Encoded, RangeFailureKind::Protocol),
        (ResponseMode::Status416, RangeFailureKind::Protocol),
    ];

    for (ordinal, (mode, expected_kind)) in cases.into_iter().enumerate() {
        let root = TempRoot::new(&format!("http-range-fault-{ordinal}"));
        let body = fixture_bytes(2_048, 5);
        let plan = sparse_plan(body.len() as u64);
        let expected = expected_range_set(&body, &plan);
        let server = RangeServer::new(body.clone(), ETAG, mode);
        let source = LocalRangeSource::new(server.url(), body.len() as u64, ETAG).unwrap();
        let workspace = RangeWorkspace::new(root.file("must-not-exist.bin")).unwrap();

        let failure = fetch_local_range_plan(
            &source,
            &plan,
            &workspace,
            &sha256(&expected),
            &limits(8, 1_024, 17),
            || false,
        )
        .unwrap_err();

        assert_eq!(failure.kind, expected_kind, "{mode:?}");
        assert!(!workspace.final_path().exists(), "{mode:?}");
        let expected_requests = if mode == ResponseMode::Corrupt {
            plan.ranges().len() as u64
        } else {
            1
        };
        assert_eq!(failure.ledger.request_count, expected_requests, "{mode:?}");
        if matches!(mode, ResponseMode::Truncate | ResponseMode::Corrupt) {
            assert!(failure.ledger.response_body_bytes > 0, "{mode:?}");
        } else {
            assert_eq!(failure.ledger.response_body_bytes, 0, "{mode:?}");
        }
        if matches!(
            expected_kind,
            RangeFailureKind::SourceChanged | RangeFailureKind::OutputChecksumMismatch
        ) {
            assert!(!workspace.part_path().exists(), "{mode:?}");
            assert!(!workspace.checkpoint_path().exists(), "{mode:?}");
            assert!(!workspace.range_directory().exists(), "{mode:?}");
        }
    }
}

#[test]
fn source_urls_validators_plans_and_transfer_budgets_fail_closed() {
    assert_eq!(
        LocalRangeSource::new("https://127.0.0.1/file", 10, ETAG),
        Err(RangeFailureKind::InvalidSource)
    );
    assert_eq!(
        LocalRangeSource::new("http://localhost:8080/file", 10, ETAG),
        Err(RangeFailureKind::InvalidSource)
    );
    assert_eq!(
        LocalRangeSource::new("http://127.0.0.1:8080/file?token=x", 10, ETAG),
        Err(RangeFailureKind::InvalidSource)
    );
    assert_eq!(
        LocalRangeSource::new("http://127.0.0.1:8080/file", 10, "W/\"weak\""),
        Err(RangeFailureKind::InvalidStrongEtag)
    );
    assert_eq!(
        RangeTransferLimits::new(0, 10, 1, 1_000),
        Err(RangeFailureKind::BudgetExceeded)
    );

    let root = TempRoot::new("http-range-budget");
    let body = fixture_bytes(2_048, 7);
    let plan = sparse_plan(body.len() as u64);
    let expected = expected_range_set(&body, &plan);
    let server = RangeServer::new(body.clone(), ETAG, ResponseMode::Normal);
    let source = LocalRangeSource::new(server.url(), body.len() as u64, ETAG).unwrap();

    for (name, limits) in [
        ("requests", limits(1, 1_024, 17)),
        ("bytes", limits(8, plan.transfer_bytes() - 1, 17)),
    ] {
        let workspace = RangeWorkspace::new(root.file(&format!("{name}.bin"))).unwrap();
        let failure = fetch_local_range_plan(
            &source,
            &plan,
            &workspace,
            &sha256(&expected),
            &limits,
            || false,
        )
        .unwrap_err();
        assert_eq!(failure.kind, RangeFailureKind::BudgetExceeded);
        assert_eq!(failure.ledger.request_count, 0);
        assert!(!workspace.final_path().exists());
    }
    assert!(server.requests().is_empty());

    let full_plan = RangePlan::build(
        body.len() as u64,
        [ByteRange::new(0, body.len() as u64, body.len() as u64).unwrap()],
        MergePolicy {
            maximum_gap_bytes: 0,
            maximum_extra_bytes: 0,
            maximum_ranges: 1,
            maximum_transfer_bytes: body.len() as u64,
        },
    )
    .unwrap();
    let workspace = RangeWorkspace::new(root.file("full.bin")).unwrap();
    let failure = fetch_local_range_plan(
        &source,
        &full_plan,
        &workspace,
        &sha256(&body),
        &limits(2, body.len() as u64, 32),
        || false,
    )
    .unwrap_err();
    assert_eq!(failure.kind, RangeFailureKind::BudgetExceeded);
}

#[test]
fn an_existing_final_is_never_treated_as_an_unverified_success() {
    let root = TempRoot::new("http-range-existing-final");
    let body = fixture_bytes(2_048, 9);
    let plan = sparse_plan(body.len() as u64);
    let expected = expected_range_set(&body, &plan);
    let server = RangeServer::new(body.clone(), ETAG, ResponseMode::Normal);
    let source = LocalRangeSource::new(server.url(), body.len() as u64, ETAG).unwrap();
    let workspace = RangeWorkspace::new(root.file("existing.bin")).unwrap();
    fs::write(workspace.final_path(), b"unverified existing bytes").unwrap();

    let failure = fetch_local_range_plan(
        &source,
        &plan,
        &workspace,
        &sha256(&expected),
        &limits(8, 1_024, 17),
        || false,
    )
    .unwrap_err();

    assert_eq!(failure.kind, RangeFailureKind::DestinationExists);
    assert_eq!(
        fs::read(workspace.final_path()).unwrap(),
        b"unverified existing bytes"
    );
    assert!(server.requests().is_empty());
}

#[test]
fn a_stale_part_hard_link_is_rejected_without_truncating_its_target() {
    let root = TempRoot::new("http-range-part-alias");
    let body = fixture_bytes(2_048, 23);
    let plan = sparse_plan(body.len() as u64);
    let expected = expected_range_set(&body, &plan);
    let server = RangeServer::new(body.clone(), ETAG, ResponseMode::Normal);
    let source = LocalRangeSource::new(server.url(), body.len() as u64, ETAG).unwrap();
    let workspace = RangeWorkspace::new(root.file("alias.bin")).unwrap();
    let protected = root.file("protected.bin");
    fs::write(&protected, b"must remain unchanged").unwrap();
    fs::hard_link(&protected, workspace.part_path()).unwrap();

    let failure = fetch_local_range_plan(
        &source,
        &plan,
        &workspace,
        &sha256(&expected),
        &limits(8, 1_024, 17),
        || false,
    )
    .unwrap_err();

    assert_eq!(failure.kind, RangeFailureKind::RecoveryInvalid);
    assert_eq!(fs::read(protected).unwrap(), b"must remain unchanged");
    assert!(server.requests().is_empty());
}

fn fixture_bytes(length: usize, seed: u8) -> Vec<u8> {
    (0..length)
        .map(|index| (index as u8).wrapping_mul(31).wrapping_add(seed))
        .collect()
}

fn sparse_plan(source_len: u64) -> RangePlan {
    RangePlan::build(
        source_len,
        [
            ByteRange::new(16, 96, source_len).unwrap(),
            ByteRange::new(768, 928, source_len).unwrap(),
        ],
        MergePolicy {
            maximum_gap_bytes: 0,
            maximum_extra_bytes: 0,
            maximum_ranges: 8,
            maximum_transfer_bytes: 1_024,
        },
    )
    .unwrap()
}

fn expected_range_set(source: &[u8], plan: &RangePlan) -> Vec<u8> {
    plan.ranges()
        .iter()
        .flat_map(|range| {
            source[range.start() as usize..range.end_exclusive() as usize]
                .iter()
                .copied()
        })
        .collect()
}

fn limits(
    maximum_requests: u64,
    maximum_response_body_bytes: u64,
    read_block_bytes: usize,
) -> RangeTransferLimits {
    RangeTransferLimits::new(
        maximum_requests,
        maximum_response_body_bytes,
        read_block_bytes,
        2_000,
    )
    .unwrap()
}

fn sha256(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}
