//! Pure deterministic segment-planning and parsed-response validation policy.
//!
//! These value objects are advisory execution inputs only. They perform no I/O,
//! carry no durable completion state, and do not select or invoke a backend.

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum RemoteLength {
    Unknown,
    Known(u64),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RangeCapability {
    Unknown,
    AdvertisedOnly,
    Proven,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum EntityTag {
    Absent,
    Weak(String),
    Strong(String),
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct LastModified(String);

impl LastModified {
    pub fn new(value: impl Into<String>) -> Self {
        Self(value.into())
    }

    pub fn as_str(&self) -> &str {
        self.0.as_str()
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum FrozenChecksumAlgorithm {
    Md5,
    Sha1,
    Sha256,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FrozenChecksumEvidence {
    pub algorithm: FrozenChecksumAlgorithm,
    pub digest: String,
    pub expected_size: Option<u64>,
}

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub enum SegmentLevel {
    One,
    Two,
    Four,
    Eight,
}

impl SegmentLevel {
    pub const fn count(self) -> u64 {
        match self {
            Self::One => 1,
            Self::Two => 2,
            Self::Four => 4,
            Self::Eight => 8,
        }
    }

    pub const fn from_count(count: u8) -> Self {
        match count {
            0 | 1 => Self::One,
            2 | 3 => Self::Two,
            4..=7 => Self::Four,
            _ => Self::Eight,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum MaximumSegmentPolicy {
    Unbounded,
    AtMost(SegmentLevel),
}

impl MaximumSegmentPolicy {
    pub(crate) fn clamp_level(self, requested_level: SegmentLevel) -> SegmentLevel {
        effective_maximum_level(requested_level, self)
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum StableObjectIdentity {
    StableStrongEtag { value: String, length: u64 },
    ConditionalLastModified { value: LastModified, length: u64 },
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum SegmentPlanContext {
    Fresh,
    SameSessionRetry {
        prior_identity: StableObjectIdentity,
    },
    ProcessRestart {
        prior_identity: StableObjectIdentity,
    },
    ResolverRefresh {
        prior_identity: StableObjectIdentity,
    },
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SegmentPlanningInput {
    pub remote_length: RemoteLength,
    pub range_capability: RangeCapability,
    pub etag: EntityTag,
    pub last_modified: Option<LastModified>,
    pub frozen_checksum: Option<FrozenChecksumEvidence>,
    pub context: SegmentPlanContext,
    pub requested_level: SegmentLevel,
    pub minimum_segment_bytes: u64,
    pub maximum_segment_policy: MaximumSegmentPolicy,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct SegmentRange {
    pub start: u64,
    pub end: u64,
}

/// An advisory, planner-produced range plan.
///
/// Its fields are deliberately private so callers cannot construct an arbitrary
/// plan or validate a range that the planner did not emit. Read-only accessors
/// expose the facts needed by future adapters.
///
/// ```compile_fail
/// use insar_download::{SegmentLevel, SegmentPlan};
///
/// let _plan = SegmentPlan {
///     total_length: 10,
///     effective_level: SegmentLevel::Two,
///     identity: todo!(),
///     frozen_checksum: None,
///     ranges: vec![],
/// };
/// ```
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SegmentPlan {
    total_length: u64,
    effective_level: SegmentLevel,
    identity: StableObjectIdentity,
    frozen_checksum: Option<FrozenChecksumEvidence>,
    ranges: Vec<SegmentRange>,
}

impl SegmentPlan {
    pub const fn total_length(&self) -> u64 {
        self.total_length
    }

    pub const fn effective_level(&self) -> SegmentLevel {
        self.effective_level
    }

    pub const fn identity(&self) -> &StableObjectIdentity {
        &self.identity
    }

    pub const fn frozen_checksum(&self) -> Option<&FrozenChecksumEvidence> {
        self.frozen_checksum.as_ref()
    }

    pub fn ranges(&self) -> &[SegmentRange] {
        &self.ranges
    }

    pub fn contains_range(&self, expected: &SegmentRange) -> bool {
        self.ranges.contains(expected)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SingleStreamReason {
    RequestedLevelOne,
    UnknownLength,
    ZeroLength,
    RangeCapabilityNotProven,
    FrozenSizeConflict,
    MissingStableIdentity,
    ResumeIdentityChanged,
    MaximumPolicyRestrictsToSingleStream,
    InsufficientMinimumSegmentBytes,
    ArithmeticOverflow,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct SingleStreamDecision {
    pub reason: SingleStreamReason,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum SegmentPlanningDecision {
    Segmented(SegmentPlan),
    SingleStream(SingleStreamDecision),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SegmentPartitionError {
    ZeroLength,
    EmptyRange,
    ArithmeticOverflow,
}

pub fn partition_half_open_ranges(
    total_length: u64,
    level: SegmentLevel,
) -> Result<Vec<SegmentRange>, SegmentPartitionError> {
    if total_length == 0 {
        return Err(SegmentPartitionError::ZeroLength);
    }

    let count = level.count();
    if total_length < count {
        return Err(SegmentPartitionError::EmptyRange);
    }

    let base = total_length / count;
    let remainder = total_length % count;
    let mut ranges = Vec::new();
    let mut start: u64 = 0;

    for index in 0..count {
        let width = base
            .checked_add(u64::from(index < remainder))
            .ok_or(SegmentPartitionError::ArithmeticOverflow)?;
        if width == 0 {
            return Err(SegmentPartitionError::EmptyRange);
        }
        let end = start
            .checked_add(width)
            .ok_or(SegmentPartitionError::ArithmeticOverflow)?;
        ranges.push(SegmentRange { start, end });
        start = end;
    }

    if start != total_length {
        return Err(SegmentPartitionError::ArithmeticOverflow);
    }

    Ok(ranges)
}

pub fn plan_segments(input: &SegmentPlanningInput) -> SegmentPlanningDecision {
    if input.requested_level == SegmentLevel::One {
        return single_stream(SingleStreamReason::RequestedLevelOne);
    }

    let total_length = match input.remote_length {
        RemoteLength::Unknown => return single_stream(SingleStreamReason::UnknownLength),
        RemoteLength::Known(0) => return single_stream(SingleStreamReason::ZeroLength),
        RemoteLength::Known(length) => length,
    };

    if input.range_capability != RangeCapability::Proven {
        return single_stream(SingleStreamReason::RangeCapabilityNotProven);
    }

    if input
        .frozen_checksum
        .as_ref()
        .and_then(|evidence| evidence.expected_size)
        .is_some_and(|expected_size| expected_size != total_length)
    {
        return single_stream(SingleStreamReason::FrozenSizeConflict);
    }

    let identity = match derive_identity(input, total_length) {
        Some(identity) => identity,
        None => return single_stream(SingleStreamReason::MissingStableIdentity),
    };

    if !context_matches(&input.context, &identity) {
        return single_stream(SingleStreamReason::ResumeIdentityChanged);
    }

    let maximum_level =
        effective_maximum_level(input.requested_level, input.maximum_segment_policy);
    for level in [SegmentLevel::Eight, SegmentLevel::Four, SegmentLevel::Two] {
        if level.count() > maximum_level.count()
            || !satisfies_minimum(total_length, level, input.minimum_segment_bytes)
        {
            continue;
        }

        match partition_half_open_ranges(total_length, level) {
            Ok(ranges) => {
                return SegmentPlanningDecision::Segmented(SegmentPlan {
                    total_length,
                    effective_level: level,
                    identity,
                    frozen_checksum: input.frozen_checksum.clone(),
                    ranges,
                });
            }
            Err(SegmentPartitionError::ArithmeticOverflow) => {
                return single_stream(SingleStreamReason::ArithmeticOverflow);
            }
            Err(SegmentPartitionError::ZeroLength | SegmentPartitionError::EmptyRange) => {}
        }
    }

    if maximum_level == SegmentLevel::One {
        single_stream(SingleStreamReason::MaximumPolicyRestrictsToSingleStream)
    } else {
        single_stream(SingleStreamReason::InsufficientMinimumSegmentBytes)
    }
}

fn single_stream(reason: SingleStreamReason) -> SegmentPlanningDecision {
    SegmentPlanningDecision::SingleStream(SingleStreamDecision { reason })
}

fn derive_identity(
    input: &SegmentPlanningInput,
    total_length: u64,
) -> Option<StableObjectIdentity> {
    match &input.etag {
        EntityTag::Strong(value) => Some(StableObjectIdentity::StableStrongEtag {
            value: value.clone(),
            length: total_length,
        }),
        EntityTag::Absent | EntityTag::Weak(_) => input.last_modified.as_ref().map(|value| {
            StableObjectIdentity::ConditionalLastModified {
                value: value.clone(),
                length: total_length,
            }
        }),
    }
}

fn context_matches(context: &SegmentPlanContext, identity: &StableObjectIdentity) -> bool {
    match context {
        SegmentPlanContext::Fresh => true,
        SegmentPlanContext::SameSessionRetry { prior_identity }
        | SegmentPlanContext::ProcessRestart { prior_identity }
        | SegmentPlanContext::ResolverRefresh { prior_identity } => prior_identity == identity,
    }
}

fn effective_maximum_level(
    requested_level: SegmentLevel,
    maximum_segment_policy: MaximumSegmentPolicy,
) -> SegmentLevel {
    match maximum_segment_policy {
        MaximumSegmentPolicy::Unbounded => requested_level,
        MaximumSegmentPolicy::AtMost(maximum_level)
            if maximum_level.count() < requested_level.count() =>
        {
            maximum_level
        }
        MaximumSegmentPolicy::AtMost(_) => requested_level,
    }
}

fn satisfies_minimum(total_length: u64, level: SegmentLevel, minimum_segment_bytes: u64) -> bool {
    let count = level.count();
    total_length >= count && total_length / count >= minimum_segment_bytes
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SegmentResponseStatus {
    PartialContent,
    FullBodyOk,
    RetryableFailure,
    TerminalFailure,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ParsedContentRange {
    pub start: u64,
    pub end_inclusive: u64,
    pub total: Option<u64>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SegmentResponseEvidence {
    pub status: SegmentResponseStatus,
    pub content_range: Option<ParsedContentRange>,
    pub content_length: Option<u64>,
    pub etag: EntityTag,
    pub last_modified: Option<LastModified>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RestartReason {
    RangeRequestReturnedFullBody,
    MissingRequiredValidator,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum IdentityConflictReason {
    ValidatorChanged,
    TotalLengthChanged,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum MalformedEvidenceReason {
    ExpectedRangeOutsidePlan,
    MissingContentRange,
    InvalidRange,
    EndOverflow,
    WrongStart,
    WrongEnd,
    MissingTotal,
    RangeExceedsTotal,
    ContentLengthMismatch,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TerminalResponseReason {
    TerminalStatus,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SegmentResponseDecision {
    AcceptSegment,
    RetrySameSegment,
    RestartNativeSingleStream(RestartReason),
    FailRemoteIdentityConflict(IdentityConflictReason),
    FailMalformedEvidence(MalformedEvidenceReason),
    FailTerminalResponse(TerminalResponseReason),
}

pub fn validate_segment_response(
    plan: &SegmentPlan,
    expected: &SegmentRange,
    evidence: &SegmentResponseEvidence,
) -> SegmentResponseDecision {
    if !plan.contains_range(expected) {
        return malformed(MalformedEvidenceReason::ExpectedRangeOutsidePlan);
    }

    match evidence.status {
        SegmentResponseStatus::RetryableFailure => {
            return SegmentResponseDecision::RetrySameSegment;
        }
        SegmentResponseStatus::FullBodyOk => {
            return SegmentResponseDecision::RestartNativeSingleStream(
                RestartReason::RangeRequestReturnedFullBody,
            );
        }
        SegmentResponseStatus::TerminalFailure => {
            return SegmentResponseDecision::FailTerminalResponse(
                TerminalResponseReason::TerminalStatus,
            );
        }
        SegmentResponseStatus::PartialContent => {}
    }

    let content_range = match evidence.content_range {
        Some(content_range) => content_range,
        None => return malformed(MalformedEvidenceReason::MissingContentRange),
    };
    if content_range.start > content_range.end_inclusive {
        return malformed(MalformedEvidenceReason::InvalidRange);
    }

    let response_end = match content_range.end_inclusive.checked_add(1) {
        Some(end) => end,
        None => return malformed(MalformedEvidenceReason::EndOverflow),
    };
    if content_range.start != expected.start {
        return malformed(MalformedEvidenceReason::WrongStart);
    }
    if response_end != expected.end {
        return malformed(MalformedEvidenceReason::WrongEnd);
    }

    let total = match content_range.total {
        Some(total) => total,
        None => return malformed(MalformedEvidenceReason::MissingTotal),
    };
    if response_end > total {
        return malformed(MalformedEvidenceReason::RangeExceedsTotal);
    }
    if total != plan.total_length {
        return SegmentResponseDecision::FailRemoteIdentityConflict(
            IdentityConflictReason::TotalLengthChanged,
        );
    }

    let expected_width = match expected.end.checked_sub(expected.start) {
        Some(width) => width,
        None => return malformed(MalformedEvidenceReason::ExpectedRangeOutsidePlan),
    };
    if evidence
        .content_length
        .is_some_and(|content_length| content_length != expected_width)
    {
        return malformed(MalformedEvidenceReason::ContentLengthMismatch);
    }

    match validator_decision(&plan.identity, evidence) {
        Some(decision) => decision,
        None => SegmentResponseDecision::AcceptSegment,
    }
}

fn malformed(reason: MalformedEvidenceReason) -> SegmentResponseDecision {
    SegmentResponseDecision::FailMalformedEvidence(reason)
}

fn validator_decision(
    identity: &StableObjectIdentity,
    evidence: &SegmentResponseEvidence,
) -> Option<SegmentResponseDecision> {
    match identity {
        StableObjectIdentity::StableStrongEtag { value, .. } => match &evidence.etag {
            EntityTag::Strong(current) if current == value => None,
            EntityTag::Strong(_) => Some(SegmentResponseDecision::FailRemoteIdentityConflict(
                IdentityConflictReason::ValidatorChanged,
            )),
            EntityTag::Absent | EntityTag::Weak(_) => {
                Some(SegmentResponseDecision::RestartNativeSingleStream(
                    RestartReason::MissingRequiredValidator,
                ))
            }
        },
        StableObjectIdentity::ConditionalLastModified { value, .. } => {
            match &evidence.last_modified {
                Some(current) if current == value => None,
                Some(_) => Some(SegmentResponseDecision::FailRemoteIdentityConflict(
                    IdentityConflictReason::ValidatorChanged,
                )),
                None => Some(SegmentResponseDecision::RestartNativeSingleStream(
                    RestartReason::MissingRequiredValidator,
                )),
            }
        }
    }
}

/// Parses an HTTP `Content-Range: bytes <start>-<end>/<total|*>` header value.
pub fn parse_content_range(header: &str) -> Option<ParsedContentRange> {
    let trimmed = header.trim();
    let rest = trimmed
        .strip_prefix("bytes ")
        .or_else(|| trimmed.strip_prefix("BYTES "))
        .or_else(|| {
            let (unit, tail) = trimmed.split_once(char::is_whitespace)?;
            unit.eq_ignore_ascii_case("bytes").then_some(tail)
        })?
        .trim();
    let (range_part, total_part) = rest.split_once('/')?;
    let (start_str, end_str) = range_part.trim().split_once('-')?;
    let start = start_str.trim().parse::<u64>().ok()?;
    let end_inclusive = end_str.trim().parse::<u64>().ok()?;
    let total = match total_part.trim() {
        "*" => None,
        raw => Some(raw.parse::<u64>().ok()?),
    };
    Some(ParsedContentRange {
        start,
        end_inclusive,
        total,
    })
}

/// Parses an HTTP `ETag` header value into a weak or strong [`EntityTag`].
pub fn parse_entity_tag(header: Option<&str>) -> EntityTag {
    let Some(raw) = header.map(str::trim).filter(|value| !value.is_empty()) else {
        return EntityTag::Absent;
    };
    if raw.starts_with("W/") || raw.starts_with("w/") {
        EntityTag::Weak(raw.to_owned())
    } else {
        EntityTag::Strong(raw.to_owned())
    }
}

/// Returns `true` when an `Accept-Ranges` header advertises byte ranges (`bytes`).
pub fn accept_ranges_supports_bytes(header: Option<&str>) -> bool {
    let Some(raw) = header else {
        return false;
    };
    raw.split(',')
        .any(|token| token.trim().eq_ignore_ascii_case("bytes"))
}
