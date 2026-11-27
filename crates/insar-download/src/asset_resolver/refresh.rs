use insar_core::StableAssetReference;

use super::{
    AuthRefreshPermit, Clock, RemoteValidator, ResolveError, ResolvedAsset, ResolvedChecksum,
    ResolvedMethod,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RefreshReason {
    Unauthorized,
    Expired,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RefreshDecision {
    RestartFromBeginning,
    ResumeFrom { offset: u64 },
}

pub struct RefreshEvaluation<'a> {
    pub reason: RefreshReason,
    pub permit: AuthRefreshPermit,
    pub previous_reference: &'a StableAssetReference,
    pub refreshed_reference: &'a StableAssetReference,
    pub previous: &'a ResolvedAsset,
    pub refreshed: &'a ResolvedAsset,
    pub completed_bytes: u64,
    pub prior_response_validator: Option<&'a RemoteValidator>,
    pub clock: &'a dyn Clock,
}

/// Evaluate whether a newly resolved request can safely continue an existing
/// transfer. This function only compares in-memory facts and consumes the
/// independent resolver refresh budget. It performs no network or file I/O.
pub fn evaluate_refresh(input: RefreshEvaluation<'_>) -> Result<RefreshDecision, ResolveError> {
    let RefreshEvaluation {
        reason,
        permit,
        previous_reference,
        refreshed_reference,
        previous,
        refreshed,
        completed_bytes,
        prior_response_validator,
        clock,
    } = input;

    let _permit = permit;
    match reason {
        RefreshReason::Unauthorized | RefreshReason::Expired => {}
    }
    previous_reference
        .validate()
        .map_err(|_| ResolveError::reference_invalid())?;
    refreshed_reference
        .validate()
        .map_err(|_| ResolveError::reference_invalid())?;
    if previous_reference != refreshed_reference {
        return Err(ResolveError::reference_invalid());
    }
    refreshed.ensure_fresh(clock)?;

    if previous.method() != ResolvedMethod::Get || refreshed.method() != ResolvedMethod::Get {
        return Err(ResolveError::failed());
    }
    ensure_optional_equal(previous.expected_size(), refreshed.expected_size())?;
    ensure_checksum_equal(previous.expected_checksum(), refreshed.expected_checksum())?;
    ensure_validators_compatible(previous.remote_validator(), refreshed.remote_validator())?;
    if let Some(prior) = prior_response_validator {
        ensure_validators_compatible(prior, refreshed.remote_validator())?;
        ensure_optional_equal(previous.expected_size(), prior.content_length)?;
        ensure_optional_equal(refreshed.expected_size(), prior.content_length)?;
    }
    ensure_internal_size_consistency(previous)?;
    ensure_internal_size_consistency(refreshed)?;

    if refreshed
        .expected_size()
        .or(refreshed.remote_validator().content_length)
        .is_some_and(|size| completed_bytes > size)
    {
        return Err(ResolveError::failed());
    }

    if completed_bytes == 0
        || !has_safe_resume_evidence(previous, refreshed, prior_response_validator)
    {
        Ok(RefreshDecision::RestartFromBeginning)
    } else {
        Ok(RefreshDecision::ResumeFrom {
            offset: completed_bytes,
        })
    }
}

fn has_safe_resume_evidence(
    previous: &ResolvedAsset,
    refreshed: &ResolvedAsset,
    prior_response_validator: Option<&RemoteValidator>,
) -> bool {
    let same_checksum = matches!(
        (previous.expected_checksum(), refreshed.expected_checksum()),
        (Some(left), Some(right))
            if left.algorithm.eq_ignore_ascii_case(&right.algorithm) && left.value == right.value
    );
    let previous_validator = previous.remote_validator();
    let refreshed_validator = refreshed.remote_validator();
    let same_etag = matches!(
        (previous_validator.etag.as_deref(), refreshed_validator.etag.as_deref()),
        (Some(left), Some(right)) if left == right
    );
    let same_last_modified = matches!(
        (
            previous_validator.last_modified.as_deref(),
            refreshed_validator.last_modified.as_deref(),
        ),
        (Some(left), Some(right)) if left == right
    );
    let prior_response_identity = prior_response_validator.is_some_and(|validator| {
        validator.etag.as_deref().is_some_and(is_strong_etag)
            || (validator.last_modified.is_some() && validator.content_length.is_some())
    });
    same_checksum || same_etag || same_last_modified || prior_response_identity
}

fn is_strong_etag(value: &str) -> bool {
    let value = value.trim();
    value.len() > 2
        && value.len() <= 1024
        && !value
            .get(..2)
            .is_some_and(|prefix| prefix.eq_ignore_ascii_case("w/"))
        && value.starts_with('"')
        && value.ends_with('"')
        && !value[1..value.len() - 1].contains('"')
        && !value.chars().any(char::is_control)
}

fn ensure_optional_equal<T: PartialEq>(
    previous: Option<T>,
    refreshed: Option<T>,
) -> Result<(), ResolveError> {
    if matches!((previous, refreshed), (Some(left), Some(right)) if left != right) {
        return Err(ResolveError::failed());
    }
    Ok(())
}

fn ensure_checksum_equal(
    previous: Option<&ResolvedChecksum>,
    refreshed: Option<&ResolvedChecksum>,
) -> Result<(), ResolveError> {
    if let (Some(previous), Some(refreshed)) = (previous, refreshed) {
        let same = previous
            .algorithm
            .eq_ignore_ascii_case(&refreshed.algorithm)
            && previous.value == refreshed.value;
        if !same {
            return Err(ResolveError::failed());
        }
    }
    Ok(())
}

fn ensure_validators_compatible(
    previous: &RemoteValidator,
    refreshed: &RemoteValidator,
) -> Result<(), ResolveError> {
    ensure_optional_equal(previous.etag.as_deref(), refreshed.etag.as_deref())?;
    ensure_optional_equal(
        previous.last_modified.as_deref(),
        refreshed.last_modified.as_deref(),
    )?;
    ensure_optional_equal(previous.content_length, refreshed.content_length)
}

fn ensure_internal_size_consistency(asset: &ResolvedAsset) -> Result<(), ResolveError> {
    ensure_optional_equal(
        asset.expected_size(),
        asset.remote_validator().content_length,
    )
}
