use insar_download::{
    partition_half_open_ranges, plan_segments, validate_segment_response, EntityTag,
    FrozenChecksumAlgorithm, FrozenChecksumEvidence, IdentityConflictReason, LastModified,
    MalformedEvidenceReason, MaximumSegmentPolicy, ParsedContentRange, RangeCapability,
    RemoteLength, RestartReason, SegmentLevel, SegmentPartitionError, SegmentPlan,
    SegmentPlanContext, SegmentPlanningDecision, SegmentPlanningInput, SegmentRange,
    SegmentResponseDecision, SegmentResponseEvidence, SegmentResponseStatus, SingleStreamReason,
    StableObjectIdentity, TerminalResponseReason,
};

fn input(length: u64, level: SegmentLevel) -> SegmentPlanningInput {
    SegmentPlanningInput {
        remote_length: RemoteLength::Known(length),
        range_capability: RangeCapability::Proven,
        etag: EntityTag::Strong("stable-etag".to_string()),
        last_modified: None,
        frozen_checksum: None,
        context: SegmentPlanContext::Fresh,
        requested_level: level,
        minimum_segment_bytes: 1,
        maximum_segment_policy: MaximumSegmentPolicy::Unbounded,
    }
}

fn segmented(input: SegmentPlanningInput) -> SegmentPlan {
    match plan_segments(&input) {
        SegmentPlanningDecision::Segmented(plan) => plan,
        decision => panic!("expected segmented plan, got {decision:?}"),
    }
}

fn single_reason(input: SegmentPlanningInput) -> SingleStreamReason {
    match plan_segments(&input) {
        SegmentPlanningDecision::SingleStream(decision) => decision.reason,
        decision => panic!("expected single stream, got {decision:?}"),
    }
}

fn strong_response(status: SegmentResponseStatus) -> SegmentResponseEvidence {
    SegmentResponseEvidence {
        status,
        content_range: Some(ParsedContentRange {
            start: 0,
            end_inclusive: 4,
            total: Some(10),
        }),
        content_length: Some(5),
        etag: EntityTag::Strong("stable-etag".to_string()),
        last_modified: None,
    }
}

fn matching_strong_response(
    expected: &SegmentRange,
    total: u64,
    status: SegmentResponseStatus,
) -> SegmentResponseEvidence {
    SegmentResponseEvidence {
        status,
        content_range: Some(ParsedContentRange {
            start: expected.start,
            end_inclusive: expected.end.checked_sub(1).unwrap(),
            total: Some(total),
        }),
        content_length: Some(expected.end - expected.start),
        etag: EntityTag::Strong("stable-etag".to_string()),
        last_modified: None,
    }
}

fn assert_partition(length: u64, level: SegmentLevel) {
    let ranges = partition_half_open_ranges(length, level).unwrap();
    assert_eq!(ranges.first().map(|range| range.start), Some(0));
    assert_eq!(ranges.last().map(|range| range.end), Some(length));
    assert!(ranges.iter().all(|range| range.start < range.end));
    for pair in ranges.windows(2) {
        assert_eq!(pair[0].end, pair[1].start);
    }
}

#[test]
fn partitions_supported_levels_and_distributes_remainders() {
    assert_eq!(
        partition_half_open_ranges(10, SegmentLevel::One).unwrap(),
        vec![SegmentRange { start: 0, end: 10 }]
    );
    assert_eq!(
        partition_half_open_ranges(10, SegmentLevel::Two).unwrap(),
        vec![
            SegmentRange { start: 0, end: 5 },
            SegmentRange { start: 5, end: 10 },
        ]
    );
    assert_eq!(
        partition_half_open_ranges(10, SegmentLevel::Four).unwrap(),
        vec![
            SegmentRange { start: 0, end: 3 },
            SegmentRange { start: 3, end: 6 },
            SegmentRange { start: 6, end: 8 },
            SegmentRange { start: 8, end: 10 },
        ]
    );
    assert_eq!(
        partition_half_open_ranges(10, SegmentLevel::Eight).unwrap(),
        vec![
            SegmentRange { start: 0, end: 2 },
            SegmentRange { start: 2, end: 4 },
            SegmentRange { start: 4, end: 5 },
            SegmentRange { start: 5, end: 6 },
            SegmentRange { start: 6, end: 7 },
            SegmentRange { start: 7, end: 8 },
            SegmentRange { start: 8, end: 9 },
            SegmentRange { start: 9, end: 10 },
        ]
    );
}

#[test]
fn partition_exhaustively_preserves_half_open_coverage() {
    for length in 1..=64 {
        for level in [
            SegmentLevel::One,
            SegmentLevel::Two,
            SegmentLevel::Four,
            SegmentLevel::Eight,
        ] {
            if length < level.count() {
                assert_eq!(
                    partition_half_open_ranges(length, level),
                    Err(SegmentPartitionError::EmptyRange)
                );
            } else {
                assert_partition(length, level);
            }
        }
    }

    assert_eq!(
        partition_half_open_ranges(0, SegmentLevel::One),
        Err(SegmentPartitionError::ZeroLength)
    );
}

#[test]
fn planner_degrades_deterministically_for_minimum_and_maximum_policy() {
    let mut constrained = input(10, SegmentLevel::Eight);
    constrained.minimum_segment_bytes = 2;
    let plan = segmented(constrained);
    assert_eq!(plan.effective_level(), SegmentLevel::Four);
    assert_partition(plan.total_length(), plan.effective_level());

    let mut max_limited = input(32, SegmentLevel::Eight);
    max_limited.maximum_segment_policy = MaximumSegmentPolicy::AtMost(SegmentLevel::Two);
    assert_eq!(segmented(max_limited).effective_level(), SegmentLevel::Two);

    let mut impossible = input(10, SegmentLevel::Eight);
    impossible.minimum_segment_bytes = 6;
    assert_eq!(
        single_reason(impossible),
        SingleStreamReason::InsufficientMinimumSegmentBytes
    );

    assert_eq!(
        single_reason(input(10, SegmentLevel::One)),
        SingleStreamReason::RequestedLevelOne
    );
}

#[test]
fn planner_handles_length_and_checked_arithmetic_boundaries() {
    let mut zero = input(0, SegmentLevel::Two);
    zero.remote_length = RemoteLength::Known(0);
    assert_eq!(single_reason(zero), SingleStreamReason::ZeroLength);

    assert_eq!(
        single_reason(input(1, SegmentLevel::Two)),
        SingleStreamReason::InsufficientMinimumSegmentBytes
    );

    let mut unknown = input(10, SegmentLevel::Two);
    unknown.remote_length = RemoteLength::Unknown;
    assert_eq!(single_reason(unknown), SingleStreamReason::UnknownLength);

    let ranges = partition_half_open_ranges(u64::MAX, SegmentLevel::Eight).unwrap();
    assert_eq!(ranges.len(), 8);
    assert_eq!(ranges[0].start, 0);
    assert_eq!(ranges.last().map(|range| range.end), Some(u64::MAX));
    assert!(ranges.iter().all(|range| range.start < range.end));
}

#[test]
fn planner_requires_proven_ranges_and_stable_identity() {
    let mut unknown_range = input(10, SegmentLevel::Two);
    unknown_range.range_capability = RangeCapability::Unknown;
    assert_eq!(
        single_reason(unknown_range),
        SingleStreamReason::RangeCapabilityNotProven
    );

    let mut advertised = input(10, SegmentLevel::Two);
    advertised.range_capability = RangeCapability::AdvertisedOnly;
    assert_eq!(
        single_reason(advertised),
        SingleStreamReason::RangeCapabilityNotProven
    );

    let mut weak = input(10, SegmentLevel::Two);
    weak.etag = EntityTag::Weak("weak".to_string());
    assert_eq!(
        single_reason(weak),
        SingleStreamReason::MissingStableIdentity
    );

    let mut conditional = input(10, SegmentLevel::Two);
    conditional.etag = EntityTag::Weak("weak".to_string());
    conditional.last_modified = Some(LastModified::new("Tue, 01 Jan 2030 00:00:00 GMT"));
    assert!(matches!(
        segmented(conditional).identity(),
        StableObjectIdentity::ConditionalLastModified { .. }
    ));

    let mut checksum_only = input(10, SegmentLevel::Two);
    checksum_only.etag = EntityTag::Absent;
    checksum_only.frozen_checksum = Some(FrozenChecksumEvidence {
        algorithm: FrozenChecksumAlgorithm::Sha256,
        digest: "fixture-digest".to_string(),
        expected_size: Some(10),
    });
    assert_eq!(
        single_reason(checksum_only),
        SingleStreamReason::MissingStableIdentity
    );

    let mut size_conflict = input(10, SegmentLevel::Two);
    size_conflict.frozen_checksum = Some(FrozenChecksumEvidence {
        algorithm: FrozenChecksumAlgorithm::Sha256,
        digest: "fixture-digest".to_string(),
        expected_size: Some(11),
    });
    assert_eq!(
        single_reason(size_conflict),
        SingleStreamReason::FrozenSizeConflict
    );
}

#[test]
fn planner_rechecks_identity_for_retry_restart_and_refresh() {
    let initial = segmented(input(10, SegmentLevel::Two));

    let mut retry = input(10, SegmentLevel::Two);
    retry.context = SegmentPlanContext::SameSessionRetry {
        prior_identity: initial.identity().clone(),
    };
    assert!(matches!(
        plan_segments(&retry),
        SegmentPlanningDecision::Segmented(_)
    ));

    let mut restart = input(10, SegmentLevel::Two);
    restart.context = SegmentPlanContext::ProcessRestart {
        prior_identity: initial.identity().clone(),
    };
    assert!(matches!(
        plan_segments(&restart),
        SegmentPlanningDecision::Segmented(_)
    ));

    let mut refresh = input(10, SegmentLevel::Two);
    refresh.context = SegmentPlanContext::ResolverRefresh {
        prior_identity: initial.identity().clone(),
    };
    refresh.etag = EntityTag::Strong("different-etag".to_string());
    assert_eq!(
        single_reason(refresh),
        SingleStreamReason::ResumeIdentityChanged
    );
}

#[test]
fn response_validation_accepts_matching_partial_evidence() {
    let plan = segmented(input(10, SegmentLevel::Two));
    assert_eq!(
        validate_segment_response(
            &plan,
            &plan.ranges()[0],
            &strong_response(SegmentResponseStatus::PartialContent),
        ),
        SegmentResponseDecision::AcceptSegment
    );

    let mut conditional_input = input(10, SegmentLevel::Two);
    conditional_input.etag = EntityTag::Absent;
    conditional_input.last_modified = Some(LastModified::new("Tue, 01 Jan 2030 00:00:00 GMT"));
    let conditional_plan = segmented(conditional_input);
    let response = SegmentResponseEvidence {
        status: SegmentResponseStatus::PartialContent,
        content_range: Some(ParsedContentRange {
            start: 0,
            end_inclusive: 4,
            total: Some(10),
        }),
        content_length: Some(5),
        etag: EntityTag::Absent,
        last_modified: Some(LastModified::new("Tue, 01 Jan 2030 00:00:00 GMT")),
    };
    assert_eq!(
        validate_segment_response(&conditional_plan, &conditional_plan.ranges()[0], &response),
        SegmentResponseDecision::AcceptSegment
    );
}

#[test]
fn response_validation_returns_all_non_acceptance_decisions() {
    let plan = segmented(input(10, SegmentLevel::Two));
    let expected = &plan.ranges()[0];

    assert_eq!(
        validate_segment_response(
            &plan,
            expected,
            &strong_response(SegmentResponseStatus::RetryableFailure),
        ),
        SegmentResponseDecision::RetrySameSegment
    );
    assert_eq!(
        validate_segment_response(
            &plan,
            expected,
            &strong_response(SegmentResponseStatus::FullBodyOk),
        ),
        SegmentResponseDecision::RestartNativeSingleStream(
            RestartReason::RangeRequestReturnedFullBody
        )
    );
    assert_eq!(
        validate_segment_response(
            &plan,
            expected,
            &strong_response(SegmentResponseStatus::TerminalFailure),
        ),
        SegmentResponseDecision::FailTerminalResponse(TerminalResponseReason::TerminalStatus)
    );

    let mut missing_range = strong_response(SegmentResponseStatus::PartialContent);
    missing_range.content_range = None;
    assert_eq!(
        validate_segment_response(&plan, expected, &missing_range),
        SegmentResponseDecision::FailMalformedEvidence(
            MalformedEvidenceReason::MissingContentRange
        )
    );

    let mut wrong_start = strong_response(SegmentResponseStatus::PartialContent);
    wrong_start.content_range.as_mut().unwrap().start = 1;
    assert_eq!(
        validate_segment_response(&plan, expected, &wrong_start),
        SegmentResponseDecision::FailMalformedEvidence(MalformedEvidenceReason::WrongStart)
    );

    let mut wrong_end = strong_response(SegmentResponseStatus::PartialContent);
    wrong_end.content_range.as_mut().unwrap().end_inclusive = 3;
    assert_eq!(
        validate_segment_response(&plan, expected, &wrong_end),
        SegmentResponseDecision::FailMalformedEvidence(MalformedEvidenceReason::WrongEnd)
    );

    let mut changed_total = strong_response(SegmentResponseStatus::PartialContent);
    changed_total.content_range.as_mut().unwrap().total = Some(11);
    assert_eq!(
        validate_segment_response(&plan, expected, &changed_total),
        SegmentResponseDecision::FailRemoteIdentityConflict(
            IdentityConflictReason::TotalLengthChanged
        )
    );

    let mut changed_validator = strong_response(SegmentResponseStatus::PartialContent);
    changed_validator.etag = EntityTag::Strong("changed".to_string());
    assert_eq!(
        validate_segment_response(&plan, expected, &changed_validator),
        SegmentResponseDecision::FailRemoteIdentityConflict(
            IdentityConflictReason::ValidatorChanged
        )
    );

    let mut weak_validator = strong_response(SegmentResponseStatus::PartialContent);
    weak_validator.etag = EntityTag::Weak("stable-etag".to_string());
    assert_eq!(
        validate_segment_response(&plan, expected, &weak_validator),
        SegmentResponseDecision::RestartNativeSingleStream(RestartReason::MissingRequiredValidator)
    );

    let mut wrong_length = strong_response(SegmentResponseStatus::PartialContent);
    wrong_length.content_length = Some(4);
    assert_eq!(
        validate_segment_response(&plan, expected, &wrong_length),
        SegmentResponseDecision::FailMalformedEvidence(
            MalformedEvidenceReason::ContentLengthMismatch
        )
    );

    let mut overflow_end = strong_response(SegmentResponseStatus::PartialContent);
    overflow_end.content_range.as_mut().unwrap().end_inclusive = u64::MAX;
    assert_eq!(
        validate_segment_response(&plan, expected, &overflow_end),
        SegmentResponseDecision::FailMalformedEvidence(MalformedEvidenceReason::EndOverflow)
    );

    let mut missing_total = strong_response(SegmentResponseStatus::PartialContent);
    missing_total.content_range.as_mut().unwrap().total = None;
    assert_eq!(
        validate_segment_response(&plan, expected, &missing_total),
        SegmentResponseDecision::FailMalformedEvidence(MalformedEvidenceReason::MissingTotal)
    );

    let mut impossible_total = strong_response(SegmentResponseStatus::PartialContent);
    impossible_total.content_range.as_mut().unwrap().total = Some(4);
    assert_eq!(
        validate_segment_response(&plan, expected, &impossible_total),
        SegmentResponseDecision::FailMalformedEvidence(MalformedEvidenceReason::RangeExceedsTotal)
    );

    let mut conditional_input = input(10, SegmentLevel::Two);
    conditional_input.etag = EntityTag::Absent;
    conditional_input.last_modified = Some(LastModified::new("Tue, 01 Jan 2030 00:00:00 GMT"));
    let conditional_plan = segmented(conditional_input);
    let mut changed_last_modified = SegmentResponseEvidence {
        status: SegmentResponseStatus::PartialContent,
        content_range: Some(ParsedContentRange {
            start: 0,
            end_inclusive: 4,
            total: Some(10),
        }),
        content_length: Some(5),
        etag: EntityTag::Absent,
        last_modified: Some(LastModified::new("Tue, 01 Jan 2030 00:00:00 GMT")),
    };
    changed_last_modified.last_modified = Some(LastModified::new("Wed, 02 Jan 2030 00:00:00 GMT"));
    assert_eq!(
        validate_segment_response(
            &conditional_plan,
            &conditional_plan.ranges()[0],
            &changed_last_modified,
        ),
        SegmentResponseDecision::FailRemoteIdentityConflict(
            IdentityConflictReason::ValidatorChanged
        )
    );
}

#[test]
fn plan_accessors_expose_immutable_facts_and_exact_membership() {
    let mut planning_input = input(10, SegmentLevel::Two);
    planning_input.frozen_checksum = Some(FrozenChecksumEvidence {
        algorithm: FrozenChecksumAlgorithm::Sha256,
        digest: "fixture-digest".to_string(),
        expected_size: Some(10),
    });
    let plan = segmented(planning_input);

    assert_eq!(plan.total_length(), 10);
    assert_eq!(plan.effective_level(), SegmentLevel::Two);
    assert_eq!(
        plan.identity(),
        &StableObjectIdentity::StableStrongEtag {
            value: "stable-etag".to_string(),
            length: 10,
        }
    );
    assert_eq!(
        plan.frozen_checksum(),
        Some(&FrozenChecksumEvidence {
            algorithm: FrozenChecksumAlgorithm::Sha256,
            digest: "fixture-digest".to_string(),
            expected_size: Some(10),
        })
    );
    assert_eq!(
        plan.ranges(),
        &[
            SegmentRange { start: 0, end: 5 },
            SegmentRange { start: 5, end: 10 },
        ]
    );
    assert!(plan.contains_range(&SegmentRange { start: 0, end: 5 }));
    assert!(!plan.contains_range(&SegmentRange { start: 1, end: 4 }));
}

#[test]
fn response_validation_rejects_unplanned_ranges_before_status_handling() {
    let plan = segmented(input(10, SegmentLevel::Two));
    let expected = SegmentRange { start: 1, end: 4 };
    let partial = matching_strong_response(&expected, 10, SegmentResponseStatus::PartialContent);

    assert_eq!(
        validate_segment_response(&plan, &expected, &partial),
        SegmentResponseDecision::FailMalformedEvidence(
            MalformedEvidenceReason::ExpectedRangeOutsidePlan
        )
    );

    let retry = SegmentResponseEvidence {
        status: SegmentResponseStatus::RetryableFailure,
        ..partial
    };
    assert_eq!(
        validate_segment_response(&plan, &expected, &retry),
        SegmentResponseDecision::FailMalformedEvidence(
            MalformedEvidenceReason::ExpectedRangeOutsidePlan
        )
    );
}

#[test]
fn every_planner_emitted_range_validates_with_matching_evidence() {
    let plan = segmented(input(10, SegmentLevel::Two));

    for expected in plan.ranges() {
        assert_eq!(
            validate_segment_response(
                &plan,
                expected,
                &matching_strong_response(
                    expected,
                    plan.total_length(),
                    SegmentResponseStatus::PartialContent,
                ),
            ),
            SegmentResponseDecision::AcceptSegment
        );
    }
}
