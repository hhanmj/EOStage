mod support;

use std::fs;

use insar_subset_lab::{
    fetch_local_range_plan, ByteRange, LocalRangeSource, MergePolicy, RangeFailureKind, RangePlan,
    RangeTransferLimits, RangeWorkspace,
};
use serde_json::Value;
use sha2::{Digest, Sha256};

use support::http::{RangeServer, ResponseMode};
use support::TempRoot;

const ETAG_V1: &str = "\"fixture-v1\"";
const ETAG_V2: &str = "\"fixture-v2\"";

#[test]
fn mid_range_cancellation_preserves_bound_state_and_resume_counts_duplicate_bytes() {
    let root = TempRoot::new("range-mid-cancel");
    let body = fixture_bytes(4_096, 11);
    let plan = sparse_plan(body.len() as u64);
    let expected = expected_range_set(&body, &plan);
    let server = RangeServer::new(body.clone(), ETAG_V1, ResponseMode::Normal);
    let source = LocalRangeSource::new(server.url(), body.len() as u64, ETAG_V1).unwrap();
    let workspace = RangeWorkspace::new(root.file("resume.bin")).unwrap();
    let limits = limits(16, 4_096, 31);

    let failure = fetch_local_range_plan(
        &source,
        &plan,
        &workspace,
        &sha256(&expected),
        &limits,
        || checkpoint_incomplete_bytes(&workspace) > 0,
    )
    .unwrap_err();

    assert_eq!(failure.kind, RangeFailureKind::Cancelled);
    let interrupted_bytes = failure.ledger.response_body_bytes;
    assert!(interrupted_bytes > 0);
    assert!(interrupted_bytes < plan.ranges()[0].len());
    assert!(!workspace.final_path().exists());
    assert!(workspace.checkpoint_path().exists());
    assert!(workspace.range_directory().exists());

    let completed = fetch_local_range_plan(
        &source,
        &plan,
        &workspace,
        &sha256(&expected),
        &limits,
        || false,
    )
    .unwrap();

    assert!(completed.resumed);
    assert_eq!(fs::read(&completed.output_path).unwrap(), expected);
    assert_eq!(
        completed.ledger.response_body_bytes,
        plan.transfer_bytes() + interrupted_bytes
    );
    assert_eq!(completed.ledger.retry_duplicate_bytes, interrupted_bytes);
    assert_eq!(
        completed.ledger.accepted_unique_bytes,
        plan.transfer_bytes()
    );
    assert_eq!(
        completed.ledger.request_count,
        plan.ranges().len() as u64 + 1
    );
    assert!(server
        .requests()
        .iter()
        .all(|request| request.if_range.as_deref() == Some(ETAG_V1)));
}

#[test]
fn source_change_purges_old_state_and_a_new_validator_restarts_from_range_zero() {
    let root = TempRoot::new("range-source-change");
    let old_body = fixture_bytes(4_096, 13);
    let new_body = fixture_bytes(4_096, 29);
    let plan = sparse_plan(old_body.len() as u64);
    let old_expected = expected_range_set(&old_body, &plan);
    let new_expected = expected_range_set(&new_body, &plan);
    let server = RangeServer::new(old_body.clone(), ETAG_V1, ResponseMode::Normal);
    let old_source = LocalRangeSource::new(server.url(), old_body.len() as u64, ETAG_V1).unwrap();
    let workspace = RangeWorkspace::new(root.file("changed.bin")).unwrap();
    let limits = limits(16, 4_096, 37);

    let cancelled = fetch_local_range_plan(
        &old_source,
        &plan,
        &workspace,
        &sha256(&old_expected),
        &limits,
        || checkpoint_completed_ranges(&workspace) >= 1,
    )
    .unwrap_err();
    assert_eq!(cancelled.kind, RangeFailureKind::Cancelled);
    assert!(workspace.checkpoint_path().exists());

    server.replace_object(new_body.clone(), ETAG_V2);
    let changed = fetch_local_range_plan(
        &old_source,
        &plan,
        &workspace,
        &sha256(&old_expected),
        &limits,
        || false,
    )
    .unwrap_err();
    assert_eq!(changed.kind, RangeFailureKind::SourceChanged);
    assert!(!workspace.final_path().exists());
    assert!(!workspace.part_path().exists());
    assert!(!workspace.checkpoint_path().exists());
    assert!(!workspace.range_directory().exists());

    let new_source = LocalRangeSource::new(server.url(), new_body.len() as u64, ETAG_V2).unwrap();
    let restarted = fetch_local_range_plan(
        &new_source,
        &plan,
        &workspace,
        &sha256(&new_expected),
        &limits,
        || false,
    )
    .unwrap();
    assert!(!restarted.resumed);
    assert_eq!(fs::read(&restarted.output_path).unwrap(), new_expected);

    let requests = server.requests();
    let changed_index = requests
        .iter()
        .position(|request| request.status == 200)
        .expect("old If-Range mismatch returns 200");
    assert_eq!(requests[changed_index].if_range.as_deref(), Some(ETAG_V1));
    assert_eq!(
        requests[changed_index + 1].range.as_deref(),
        Some("bytes=32-159")
    );
    assert_eq!(
        requests[changed_index + 1].if_range.as_deref(),
        Some(ETAG_V2)
    );
}

#[test]
fn cancellation_after_all_ranges_still_requires_resume_before_promotion() {
    let root = TempRoot::new("range-pre-promotion-cancel");
    let body = fixture_bytes(4_096, 17);
    let plan = sparse_plan(body.len() as u64);
    let expected = expected_range_set(&body, &plan);
    let server = RangeServer::new(body.clone(), ETAG_V1, ResponseMode::Normal);
    let source = LocalRangeSource::new(server.url(), body.len() as u64, ETAG_V1).unwrap();
    let workspace = RangeWorkspace::new(root.file("promote.bin")).unwrap();
    let limits = limits(16, 4_096, 41);

    let failure = fetch_local_range_plan(
        &source,
        &plan,
        &workspace,
        &sha256(&expected),
        &limits,
        || checkpoint_completed_ranges(&workspace) == plan.ranges().len(),
    )
    .unwrap_err();
    assert_eq!(failure.kind, RangeFailureKind::Cancelled);
    assert!(!workspace.final_path().exists());
    let request_count = server.requests().len();

    let completed = fetch_local_range_plan(
        &source,
        &plan,
        &workspace,
        &sha256(&expected),
        &limits,
        || false,
    )
    .unwrap();
    assert!(completed.resumed);
    assert_eq!(server.requests().len(), request_count);
    assert_eq!(fs::read(&completed.output_path).unwrap(), expected);
}

#[test]
fn cancellation_during_local_assembly_resumes_without_an_http_retry() {
    let root = TempRoot::new("range-assembly-cancel");
    let body = fixture_bytes(4_096, 31);
    let plan = sparse_plan(body.len() as u64);
    let expected = expected_range_set(&body, &plan);
    let server = RangeServer::new(body.clone(), ETAG_V1, ResponseMode::Normal);
    let source = LocalRangeSource::new(server.url(), body.len() as u64, ETAG_V1).unwrap();
    let workspace = RangeWorkspace::new(root.file("assembly.bin")).unwrap();
    let limits = limits(16, 4_096, 47);

    let failure = fetch_local_range_plan(
        &source,
        &plan,
        &workspace,
        &sha256(&expected),
        &limits,
        || fs::metadata(workspace.part_path()).is_ok_and(|metadata| metadata.len() > 0),
    )
    .unwrap_err();
    assert_eq!(failure.kind, RangeFailureKind::Cancelled);
    assert!(!workspace.final_path().exists());
    assert!(workspace.part_path().exists());
    let request_count = server.requests().len();

    let completed = fetch_local_range_plan(
        &source,
        &plan,
        &workspace,
        &sha256(&expected),
        &limits,
        || false,
    )
    .unwrap();
    assert!(completed.resumed);
    assert_eq!(server.requests().len(), request_count);
    assert_eq!(fs::read(&completed.output_path).unwrap(), expected);
}

#[test]
fn tampered_checkpoint_is_rejected_without_a_request_or_final() {
    let root = TempRoot::new("range-checkpoint-tamper");
    let body = fixture_bytes(4_096, 19);
    let plan = sparse_plan(body.len() as u64);
    let expected = expected_range_set(&body, &plan);
    let server = RangeServer::new(body.clone(), ETAG_V1, ResponseMode::Normal);
    let source = LocalRangeSource::new(server.url(), body.len() as u64, ETAG_V1).unwrap();
    let workspace = RangeWorkspace::new(root.file("tampered.bin")).unwrap();
    let limits = limits(16, 4_096, 43);

    let cancelled = fetch_local_range_plan(
        &source,
        &plan,
        &workspace,
        &sha256(&expected),
        &limits,
        || checkpoint_completed_ranges(&workspace) >= 1,
    )
    .unwrap_err();
    assert_eq!(cancelled.kind, RangeFailureKind::Cancelled);
    let request_count = server.requests().len();
    let mut state = checkpoint(&workspace);
    state["plan_sha256"] = Value::String("0".repeat(64));
    fs::write(
        workspace.checkpoint_path(),
        serde_json::to_vec_pretty(&state).unwrap(),
    )
    .unwrap();

    let failure = fetch_local_range_plan(
        &source,
        &plan,
        &workspace,
        &sha256(&expected),
        &limits,
        || false,
    )
    .unwrap_err();
    assert_eq!(failure.kind, RangeFailureKind::RecoveryInvalid);
    assert_eq!(server.requests().len(), request_count);
    assert!(!workspace.final_path().exists());
}

fn checkpoint(workspace: &RangeWorkspace) -> Value {
    serde_json::from_slice(&fs::read(workspace.checkpoint_path()).unwrap()).unwrap()
}

fn checkpoint_incomplete_bytes(workspace: &RangeWorkspace) -> u64 {
    if !workspace.checkpoint_path().exists() {
        return 0;
    }
    checkpoint(workspace)["incomplete_range_bytes"]
        .as_object()
        .map(|items| items.values().filter_map(Value::as_u64).sum())
        .unwrap_or(0)
}

fn checkpoint_completed_ranges(workspace: &RangeWorkspace) -> usize {
    if !workspace.checkpoint_path().exists() {
        return 0;
    }
    checkpoint(workspace)["completed_ranges"]
        .as_array()
        .map_or(0, Vec::len)
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
            ByteRange::new(32, 160, source_len).unwrap(),
            ByteRange::new(1_024, 1_280, source_len).unwrap(),
        ],
        MergePolicy {
            maximum_gap_bytes: 0,
            maximum_extra_bytes: 0,
            maximum_ranges: 8,
            maximum_transfer_bytes: 2_048,
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
