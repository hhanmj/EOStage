use std::collections::{BTreeMap, BTreeSet};
use std::fmt;
use std::fs::{self, File, OpenOptions};
use std::io::{Read, Write};
use std::net::IpAddr;
use std::path::{Path, PathBuf};
use std::time::Duration;

use reqwest::blocking::{Client, Response};
use reqwest::header::{
    HeaderMap, HeaderName, ACCEPT_ENCODING, CONTENT_ENCODING, CONTENT_LENGTH, CONTENT_RANGE, ETAG,
    IF_RANGE, RANGE,
};
use reqwest::{StatusCode, Url};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::{ByteRange, RangePlan};

const CHECKPOINT_SCHEMA: &str = "NisarLocalRangeCheckpointV1";
const MAX_CHECKPOINT_BYTES: u64 = 1024 * 1024;
const MAX_ETAG_BYTES: usize = 256;
const MAX_READ_BLOCK_BYTES: usize = 1024 * 1024;
const MAX_TIMEOUT_MILLIS: u64 = 60_000;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RangeFailureKind {
    InvalidSource,
    InvalidStrongEtag,
    InvalidOutputChecksum,
    BudgetExceeded,
    DestinationExists,
    Protocol,
    SourceChanged,
    Cancelled,
    RecoveryInvalid,
    OutputChecksumMismatch,
    LocalIo,
}

impl fmt::Display for RangeFailureKind {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::InvalidSource => "invalid local Range source",
            Self::InvalidStrongEtag => "invalid strong ETag",
            Self::InvalidOutputChecksum => "invalid expected output checksum",
            Self::BudgetExceeded => "Range transfer budget exceeded",
            Self::DestinationExists => "unverified final destination already exists",
            Self::Protocol => "strict HTTP Range protocol rejected the response",
            Self::SourceChanged => "Range source validator changed",
            Self::Cancelled => "Range transfer cancelled",
            Self::RecoveryInvalid => "Range recovery state is invalid",
            Self::OutputChecksumMismatch => "assembled Range output checksum mismatch",
            Self::LocalIo => "local Range workspace operation failed",
        })
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RangeTransferFailure {
    pub kind: RangeFailureKind,
    pub ledger: ByteLedger,
}

impl fmt::Display for RangeTransferFailure {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.kind.fmt(formatter)
    }
}

impl std::error::Error for RangeTransferFailure {}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LocalRangeSource {
    url: Url,
    url_sha256: String,
    source_len: u64,
    strong_etag: String,
}

impl LocalRangeSource {
    pub fn new(url: &str, source_len: u64, strong_etag: &str) -> Result<Self, RangeFailureKind> {
        let url = Url::parse(url).map_err(|_| RangeFailureKind::InvalidSource)?;
        let host = url
            .host_str()
            .and_then(|value| value.parse::<IpAddr>().ok())
            .filter(IpAddr::is_loopback);
        if url.scheme() != "http"
            || host.is_none()
            || !url.username().is_empty()
            || url.password().is_some()
            || url.query().is_some()
            || url.fragment().is_some()
            || source_len == 0
        {
            return Err(RangeFailureKind::InvalidSource);
        }
        validate_strong_etag(strong_etag)?;
        Ok(Self {
            url_sha256: format!("{:x}", Sha256::digest(url.as_str().as_bytes())),
            url,
            source_len,
            strong_etag: strong_etag.to_owned(),
        })
    }

    pub fn url(&self) -> &str {
        self.url.as_str()
    }

    pub fn source_len(&self) -> u64 {
        self.source_len
    }

    pub fn url_sha256(&self) -> &str {
        &self.url_sha256
    }

    pub fn strong_etag(&self) -> &str {
        &self.strong_etag
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RangeTransferLimits {
    maximum_requests: u64,
    maximum_response_body_bytes: u64,
    read_block_bytes: usize,
    timeout: Duration,
}

impl RangeTransferLimits {
    pub fn new(
        maximum_requests: u64,
        maximum_response_body_bytes: u64,
        read_block_bytes: usize,
        timeout_millis: u64,
    ) -> Result<Self, RangeFailureKind> {
        if maximum_requests == 0
            || maximum_response_body_bytes == 0
            || read_block_bytes == 0
            || read_block_bytes > MAX_READ_BLOCK_BYTES
            || timeout_millis == 0
            || timeout_millis > MAX_TIMEOUT_MILLIS
        {
            return Err(RangeFailureKind::BudgetExceeded);
        }
        Ok(Self {
            maximum_requests,
            maximum_response_body_bytes,
            read_block_bytes,
            timeout: Duration::from_millis(timeout_millis),
        })
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RangeWorkspace {
    final_path: PathBuf,
    part_path: PathBuf,
    checkpoint_path: PathBuf,
    checkpoint_temp_path: PathBuf,
    range_directory: PathBuf,
}

impl RangeWorkspace {
    pub fn new(final_path: PathBuf) -> Result<Self, RangeFailureKind> {
        let parent = final_path
            .parent()
            .filter(|parent| !parent.as_os_str().is_empty())
            .ok_or(RangeFailureKind::InvalidSource)?;
        let name = final_path
            .file_name()
            .and_then(|value| value.to_str())
            .filter(|value| !value.is_empty() && value.len() <= 240)
            .ok_or(RangeFailureKind::InvalidSource)?;
        Ok(Self {
            final_path: final_path.clone(),
            part_path: parent.join(format!("{name}.part")),
            checkpoint_path: parent.join(format!("{name}.range-state.json")),
            checkpoint_temp_path: parent.join(format!("{name}.range-state.json.tmp")),
            range_directory: parent.join(format!("{name}.ranges")),
        })
    }

    pub fn final_path(&self) -> &Path {
        &self.final_path
    }

    pub fn part_path(&self) -> &Path {
        &self.part_path
    }

    pub fn checkpoint_path(&self) -> &Path {
        &self.checkpoint_path
    }

    pub fn range_directory(&self) -> &Path {
        &self.range_directory
    }

    fn range_part_path(&self, index: usize) -> PathBuf {
        self.range_directory.join(format!("{index:05}.part"))
    }

    fn range_complete_path(&self, index: usize) -> PathBuf {
        self.range_directory.join(format!("{index:05}.range"))
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ByteLedger {
    pub planned_selected_bytes: u64,
    pub planned_transfer_bytes: u64,
    pub request_count: u64,
    pub response_body_bytes: u64,
    pub accepted_unique_bytes: u64,
    pub retry_duplicate_bytes: u64,
}

impl ByteLedger {
    fn for_plan(plan: &RangePlan) -> Self {
        Self {
            planned_selected_bytes: plan.selected_bytes(),
            planned_transfer_bytes: plan.transfer_bytes(),
            request_count: 0,
            response_body_bytes: 0,
            accepted_unique_bytes: 0,
            retry_duplicate_bytes: 0,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct VerifiedRangeSet {
    pub output_path: PathBuf,
    pub output_sha256: String,
    pub strong_etag: String,
    pub plan_sha256: String,
    pub ledger: ByteLedger,
    pub resumed: bool,
    pub workspace_cleanup_complete: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct CompletedRange {
    index: usize,
    start: u64,
    end_exclusive: u64,
    sha256: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct RangeCheckpoint {
    schema: String,
    source_url_sha256: String,
    source_len: u64,
    strong_etag: String,
    plan_sha256: String,
    expected_output_sha256: String,
    ledger: ByteLedger,
    completed_ranges: Vec<CompletedRange>,
    incomplete_range_bytes: BTreeMap<usize, u64>,
}

pub fn fetch_local_range_plan(
    source: &LocalRangeSource,
    plan: &RangePlan,
    workspace: &RangeWorkspace,
    expected_output_sha256: &str,
    limits: &RangeTransferLimits,
    mut cancelled: impl FnMut() -> bool,
) -> Result<VerifiedRangeSet, RangeTransferFailure> {
    let initial_ledger = ByteLedger::for_plan(plan);
    let fail_initial = |kind| RangeTransferFailure {
        kind,
        ledger: initial_ledger.clone(),
    };
    if plan.validate().is_err() || plan.source_len() != source.source_len {
        return Err(fail_initial(RangeFailureKind::InvalidSource));
    }
    if validate_sha256(expected_output_sha256).is_err() {
        return Err(fail_initial(RangeFailureKind::InvalidOutputChecksum));
    }
    if plan.transfer_bytes() >= source.source_len
        || plan.ranges().len() as u64 > limits.maximum_requests
        || plan.transfer_bytes() > limits.maximum_response_body_bytes
    {
        return Err(fail_initial(RangeFailureKind::BudgetExceeded));
    }
    if workspace.final_path.exists() {
        return Err(fail_initial(RangeFailureKind::DestinationExists));
    }
    let parent = workspace
        .final_path
        .parent()
        .ok_or_else(|| fail_initial(RangeFailureKind::InvalidSource))?;
    fs::create_dir_all(parent).map_err(|_| fail_initial(RangeFailureKind::LocalIo))?;

    let plan_sha256 = plan_sha256(plan).map_err(&fail_initial)?;
    let mut checkpoint = if workspace.checkpoint_path.exists() {
        load_checkpoint(workspace).map_err(&fail_initial)?
    } else {
        if workspace.part_path.exists()
            || workspace.range_directory.exists()
            || workspace.checkpoint_temp_path.exists()
        {
            return Err(fail_initial(RangeFailureKind::RecoveryInvalid));
        }
        fs::create_dir(&workspace.range_directory)
            .map_err(|_| fail_initial(RangeFailureKind::LocalIo))?;
        let checkpoint = RangeCheckpoint {
            schema: CHECKPOINT_SCHEMA.to_owned(),
            source_url_sha256: source.url_sha256.clone(),
            source_len: source.source_len,
            strong_etag: source.strong_etag.clone(),
            plan_sha256: plan_sha256.clone(),
            expected_output_sha256: expected_output_sha256.to_owned(),
            ledger: initial_ledger.clone(),
            completed_ranges: Vec::new(),
            incomplete_range_bytes: BTreeMap::new(),
        };
        persist_checkpoint(workspace, &checkpoint).map_err(&fail_initial)?;
        checkpoint
    };

    if checkpoint.strong_etag != source.strong_etag {
        purge_workspace(workspace).map_err(|_| fail_initial(RangeFailureKind::LocalIo))?;
        fs::create_dir(&workspace.range_directory)
            .map_err(|_| fail_initial(RangeFailureKind::LocalIo))?;
        checkpoint = RangeCheckpoint {
            schema: CHECKPOINT_SCHEMA.to_owned(),
            source_url_sha256: source.url_sha256.clone(),
            source_len: source.source_len,
            strong_etag: source.strong_etag.clone(),
            plan_sha256: plan_sha256.clone(),
            expected_output_sha256: expected_output_sha256.to_owned(),
            ledger: initial_ledger,
            completed_ranges: Vec::new(),
            incomplete_range_bytes: BTreeMap::new(),
        };
        persist_checkpoint(workspace, &checkpoint).map_err(|kind| failure(kind, &checkpoint))?;
    }
    validate_checkpoint(
        workspace,
        &checkpoint,
        source,
        plan,
        &plan_sha256,
        expected_output_sha256,
        limits,
    )
    .map_err(|kind| failure(kind, &checkpoint))?;
    let resumed = checkpoint.ledger.request_count > 0
        || checkpoint.ledger.response_body_bytes > 0
        || !checkpoint.completed_ranges.is_empty()
        || !checkpoint.incomplete_range_bytes.is_empty();

    let client = Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .no_proxy()
        .timeout(limits.timeout)
        .connect_timeout(limits.timeout)
        .build()
        .map_err(|_| failure(RangeFailureKind::Protocol, &checkpoint))?;

    for (index, byte_range) in plan.ranges().iter().copied().enumerate() {
        if checkpoint
            .completed_ranges
            .iter()
            .any(|completed| completed.index == index)
        {
            continue;
        }
        if cancelled() {
            return Err(failure(RangeFailureKind::Cancelled, &checkpoint));
        }
        if let Some(discarded) = checkpoint.incomplete_range_bytes.remove(&index) {
            checkpoint.ledger.retry_duplicate_bytes = checkpoint
                .ledger
                .retry_duplicate_bytes
                .checked_add(discarded)
                .ok_or_else(|| failure(RangeFailureKind::BudgetExceeded, &checkpoint))?;
            remove_file_if_present(&workspace.range_part_path(index))
                .map_err(|_| failure(RangeFailureKind::LocalIo, &checkpoint))?;
            persist_checkpoint(workspace, &checkpoint)
                .map_err(|kind| failure(kind, &checkpoint))?;
        }
        if checkpoint.ledger.request_count >= limits.maximum_requests {
            return Err(failure(RangeFailureKind::BudgetExceeded, &checkpoint));
        }
        if cancelled() {
            return Err(failure(RangeFailureKind::Cancelled, &checkpoint));
        }
        checkpoint.ledger.request_count += 1;
        persist_checkpoint(workspace, &checkpoint).map_err(|kind| failure(kind, &checkpoint))?;

        let response = client
            .get(source.url.clone())
            .header(ACCEPT_ENCODING, "identity")
            .header(IF_RANGE, source.strong_etag.as_str())
            .header(
                RANGE,
                format!(
                    "bytes={}-{}",
                    byte_range.start(),
                    byte_range.end_exclusive() - 1
                ),
            )
            .send()
            .map_err(|_| failure(RangeFailureKind::Protocol, &checkpoint))?;

        if response.status() == StatusCode::OK {
            let ledger = checkpoint.ledger.clone();
            purge_workspace(workspace).map_err(|_| RangeTransferFailure {
                kind: RangeFailureKind::LocalIo,
                ledger: ledger.clone(),
            })?;
            return Err(RangeTransferFailure {
                kind: RangeFailureKind::SourceChanged,
                ledger,
            });
        }
        if let Err(kind) = validate_response_headers(&response, source, byte_range) {
            if kind == RangeFailureKind::SourceChanged {
                let ledger = checkpoint.ledger.clone();
                drop(response);
                purge_workspace(workspace).map_err(|_| RangeTransferFailure {
                    kind: RangeFailureKind::LocalIo,
                    ledger: ledger.clone(),
                })?;
                return Err(RangeTransferFailure { kind, ledger });
            }
            return Err(failure(kind, &checkpoint));
        }
        receive_range(
            response,
            workspace,
            &mut checkpoint,
            index,
            byte_range,
            limits,
            &mut cancelled,
        )?;
    }

    if cancelled() {
        return Err(failure(RangeFailureKind::Cancelled, &checkpoint));
    }
    if let Err(assembly_failure) = assemble_verified_output(
        workspace,
        &checkpoint,
        plan,
        expected_output_sha256,
        &mut cancelled,
    ) {
        if assembly_failure.kind == RangeFailureKind::OutputChecksumMismatch {
            let ledger = assembly_failure.ledger.clone();
            purge_workspace(workspace).map_err(|_| RangeTransferFailure {
                kind: RangeFailureKind::LocalIo,
                ledger: ledger.clone(),
            })?;
            return Err(RangeTransferFailure {
                kind: RangeFailureKind::OutputChecksumMismatch,
                ledger,
            });
        }
        return Err(assembly_failure);
    }
    let mut result = VerifiedRangeSet {
        output_path: workspace.final_path.clone(),
        output_sha256: expected_output_sha256.to_owned(),
        strong_etag: source.strong_etag.clone(),
        plan_sha256,
        ledger: checkpoint.ledger.clone(),
        resumed,
        workspace_cleanup_complete: true,
    };
    let checkpoint_removed = remove_file_if_present(&workspace.checkpoint_path).is_ok();
    let checkpoint_temp_removed = remove_file_if_present(&workspace.checkpoint_temp_path).is_ok();
    let ranges_removed = fs::remove_dir_all(&workspace.range_directory).is_ok();
    result.workspace_cleanup_complete =
        checkpoint_removed && checkpoint_temp_removed && ranges_removed;
    Ok(result)
}

fn validate_response_headers(
    response: &Response,
    source: &LocalRangeSource,
    byte_range: ByteRange,
) -> Result<(), RangeFailureKind> {
    if response.status() != StatusCode::PARTIAL_CONTENT {
        return Err(RangeFailureKind::Protocol);
    }
    let etag = single_header(response.headers(), ETAG)?;
    validate_strong_etag(etag).map_err(|_| RangeFailureKind::Protocol)?;
    if etag != source.strong_etag {
        return Err(RangeFailureKind::SourceChanged);
    }
    let content_range = single_header(response.headers(), CONTENT_RANGE)?;
    let (start, end_inclusive, total) = parse_content_range(content_range)?;
    if start != byte_range.start()
        || end_inclusive != byte_range.end_exclusive() - 1
        || total != source.source_len
    {
        return Err(RangeFailureKind::Protocol);
    }
    let content_length = parse_ascii_u64(single_header(response.headers(), CONTENT_LENGTH)?)?;
    if content_length != byte_range.len() {
        return Err(RangeFailureKind::Protocol);
    }
    let encodings = response.headers().get_all(CONTENT_ENCODING);
    let encodings = encodings.iter().collect::<Vec<_>>();
    if encodings.len() > 1
        || encodings
            .first()
            .is_some_and(|value| value.to_str().ok() != Some("identity"))
    {
        return Err(RangeFailureKind::Protocol);
    }
    Ok(())
}

fn receive_range(
    mut response: Response,
    workspace: &RangeWorkspace,
    checkpoint: &mut RangeCheckpoint,
    index: usize,
    byte_range: ByteRange,
    limits: &RangeTransferLimits,
    cancelled: &mut impl FnMut() -> bool,
) -> Result<(), RangeTransferFailure> {
    let part_path = workspace.range_part_path(index);
    let complete_path = workspace.range_complete_path(index);
    if complete_path.exists() {
        return Err(failure(RangeFailureKind::RecoveryInvalid, checkpoint));
    }
    remove_file_if_present(&part_path)
        .map_err(|_| failure(RangeFailureKind::LocalIo, checkpoint))?;
    let mut part = OpenOptions::new()
        .create_new(true)
        .write(true)
        .open(&part_path)
        .map_err(|_| failure(RangeFailureKind::LocalIo, checkpoint))?;
    let mut digest = Sha256::new();
    let mut received = 0_u64;
    let mut buffer = vec![0_u8; limits.read_block_bytes];
    loop {
        if cancelled() {
            return Err(failure(RangeFailureKind::Cancelled, checkpoint));
        }
        let count = response
            .read(&mut buffer)
            .map_err(|_| failure(RangeFailureKind::Protocol, checkpoint))?;
        if count == 0 {
            break;
        }
        received = received
            .checked_add(count as u64)
            .ok_or_else(|| failure(RangeFailureKind::BudgetExceeded, checkpoint))?;
        checkpoint.ledger.response_body_bytes = checkpoint
            .ledger
            .response_body_bytes
            .checked_add(count as u64)
            .ok_or_else(|| failure(RangeFailureKind::BudgetExceeded, checkpoint))?;
        checkpoint.incomplete_range_bytes.insert(index, received);
        part.write_all(&buffer[..count])
            .map_err(|_| failure(RangeFailureKind::LocalIo, checkpoint))?;
        digest.update(&buffer[..count]);
        persist_checkpoint(workspace, checkpoint).map_err(|kind| failure(kind, checkpoint))?;
        if received > byte_range.len()
            || checkpoint.ledger.response_body_bytes > limits.maximum_response_body_bytes
        {
            return Err(failure(RangeFailureKind::BudgetExceeded, checkpoint));
        }
    }
    if received != byte_range.len() {
        return Err(failure(RangeFailureKind::Protocol, checkpoint));
    }
    part.flush()
        .and_then(|_| part.sync_all())
        .map_err(|_| failure(RangeFailureKind::LocalIo, checkpoint))?;
    drop(part);
    fs::rename(&part_path, &complete_path)
        .map_err(|_| failure(RangeFailureKind::LocalIo, checkpoint))?;
    checkpoint.incomplete_range_bytes.remove(&index);
    checkpoint.ledger.accepted_unique_bytes = checkpoint
        .ledger
        .accepted_unique_bytes
        .checked_add(received)
        .ok_or_else(|| failure(RangeFailureKind::BudgetExceeded, checkpoint))?;
    checkpoint.completed_ranges.push(CompletedRange {
        index,
        start: byte_range.start(),
        end_exclusive: byte_range.end_exclusive(),
        sha256: format!("{:x}", digest.finalize()),
    });
    checkpoint
        .completed_ranges
        .sort_unstable_by_key(|completed| completed.index);
    persist_checkpoint(workspace, checkpoint).map_err(|kind| failure(kind, checkpoint))
}

fn assemble_verified_output(
    workspace: &RangeWorkspace,
    checkpoint: &RangeCheckpoint,
    plan: &RangePlan,
    expected_output_sha256: &str,
    cancelled: &mut impl FnMut() -> bool,
) -> Result<(), RangeTransferFailure> {
    if checkpoint.completed_ranges.len() != plan.ranges().len() {
        return Err(failure(RangeFailureKind::RecoveryInvalid, checkpoint));
    }
    remove_file_if_present(&workspace.part_path)
        .map_err(|_| failure(RangeFailureKind::LocalIo, checkpoint))?;
    let mut output = OpenOptions::new()
        .create_new(true)
        .write(true)
        .open(&workspace.part_path)
        .map_err(|_| failure(RangeFailureKind::LocalIo, checkpoint))?;
    let mut buffer = [0_u8; 64 * 1024];
    for index in 0..plan.ranges().len() {
        let mut input = File::open(workspace.range_complete_path(index))
            .map_err(|_| failure(RangeFailureKind::RecoveryInvalid, checkpoint))?;
        loop {
            if cancelled() {
                return Err(failure(RangeFailureKind::Cancelled, checkpoint));
            }
            let count = input
                .read(&mut buffer)
                .map_err(|_| failure(RangeFailureKind::LocalIo, checkpoint))?;
            if count == 0 {
                break;
            }
            output
                .write_all(&buffer[..count])
                .map_err(|_| failure(RangeFailureKind::LocalIo, checkpoint))?;
        }
    }
    output
        .flush()
        .and_then(|_| output.sync_all())
        .map_err(|_| failure(RangeFailureKind::LocalIo, checkpoint))?;
    drop(output);
    let (size, digest) = file_sha256(&workspace.part_path)
        .map_err(|_| failure(RangeFailureKind::LocalIo, checkpoint))?;
    if size != plan.transfer_bytes() || digest != expected_output_sha256 {
        return Err(failure(
            RangeFailureKind::OutputChecksumMismatch,
            checkpoint,
        ));
    }
    if cancelled() {
        return Err(failure(RangeFailureKind::Cancelled, checkpoint));
    }
    if workspace.final_path.exists() {
        return Err(failure(RangeFailureKind::DestinationExists, checkpoint));
    }
    fs::rename(&workspace.part_path, &workspace.final_path)
        .map_err(|_| failure(RangeFailureKind::LocalIo, checkpoint))
}

fn validate_checkpoint(
    workspace: &RangeWorkspace,
    checkpoint: &RangeCheckpoint,
    source: &LocalRangeSource,
    plan: &RangePlan,
    plan_sha256: &str,
    expected_output_sha256: &str,
    limits: &RangeTransferLimits,
) -> Result<(), RangeFailureKind> {
    if !directory_is_real(&workspace.range_directory)
        || workspace.checkpoint_temp_path.exists()
        || checkpoint.schema != CHECKPOINT_SCHEMA
        || checkpoint.source_url_sha256 != source.url_sha256
        || checkpoint.source_len != source.source_len
        || checkpoint.strong_etag != source.strong_etag
        || checkpoint.plan_sha256 != plan_sha256
        || checkpoint.expected_output_sha256 != expected_output_sha256
        || checkpoint.ledger.planned_selected_bytes != plan.selected_bytes()
        || checkpoint.ledger.planned_transfer_bytes != plan.transfer_bytes()
        || checkpoint.ledger.request_count > limits.maximum_requests
        || checkpoint.ledger.response_body_bytes > limits.maximum_response_body_bytes
    {
        return Err(RangeFailureKind::RecoveryInvalid);
    }
    let mut indices = BTreeSet::new();
    let mut accepted = 0_u64;
    for completed in &checkpoint.completed_ranges {
        let Some(expected) = plan.ranges().get(completed.index) else {
            return Err(RangeFailureKind::RecoveryInvalid);
        };
        if !indices.insert(completed.index)
            || completed.start != expected.start()
            || completed.end_exclusive != expected.end_exclusive()
            || validate_sha256(&completed.sha256).is_err()
        {
            return Err(RangeFailureKind::RecoveryInvalid);
        }
        let complete_path = workspace.range_complete_path(completed.index);
        if !regular_file_is_real(&complete_path) {
            return Err(RangeFailureKind::RecoveryInvalid);
        }
        let (size, digest) =
            file_sha256(&complete_path).map_err(|_| RangeFailureKind::RecoveryInvalid)?;
        if size != expected.len() || digest != completed.sha256 {
            return Err(RangeFailureKind::RecoveryInvalid);
        }
        accepted = accepted
            .checked_add(size)
            .ok_or(RangeFailureKind::RecoveryInvalid)?;
    }
    if accepted != checkpoint.ledger.accepted_unique_bytes {
        return Err(RangeFailureKind::RecoveryInvalid);
    }
    let mut incomplete_total = 0_u64;
    for (index, bytes) in &checkpoint.incomplete_range_bytes {
        let Some(expected) = plan.ranges().get(*index) else {
            return Err(RangeFailureKind::RecoveryInvalid);
        };
        if indices.contains(index) || *bytes == 0 || *bytes > expected.len() {
            return Err(RangeFailureKind::RecoveryInvalid);
        }
        let part_path = workspace.range_part_path(*index);
        if !regular_file_is_real(&part_path) {
            return Err(RangeFailureKind::RecoveryInvalid);
        }
        let actual = fs::metadata(part_path)
            .map_err(|_| RangeFailureKind::RecoveryInvalid)?
            .len();
        if actual != *bytes {
            return Err(RangeFailureKind::RecoveryInvalid);
        }
        incomplete_total = incomplete_total
            .checked_add(*bytes)
            .ok_or(RangeFailureKind::RecoveryInvalid)?;
    }
    if checkpoint.ledger.response_body_bytes
        != checkpoint
            .ledger
            .accepted_unique_bytes
            .checked_add(checkpoint.ledger.retry_duplicate_bytes)
            .and_then(|value| value.checked_add(incomplete_total))
            .ok_or(RangeFailureKind::RecoveryInvalid)?
    {
        return Err(RangeFailureKind::RecoveryInvalid);
    }
    Ok(())
}

fn load_checkpoint(workspace: &RangeWorkspace) -> Result<RangeCheckpoint, RangeFailureKind> {
    if !regular_file_is_real(&workspace.checkpoint_path) {
        return Err(RangeFailureKind::RecoveryInvalid);
    }
    let metadata =
        fs::metadata(&workspace.checkpoint_path).map_err(|_| RangeFailureKind::RecoveryInvalid)?;
    if metadata.len() == 0 || metadata.len() > MAX_CHECKPOINT_BYTES {
        return Err(RangeFailureKind::RecoveryInvalid);
    }
    let bytes =
        fs::read(&workspace.checkpoint_path).map_err(|_| RangeFailureKind::RecoveryInvalid)?;
    serde_json::from_slice(&bytes).map_err(|_| RangeFailureKind::RecoveryInvalid)
}

fn persist_checkpoint(
    workspace: &RangeWorkspace,
    checkpoint: &RangeCheckpoint,
) -> Result<(), RangeFailureKind> {
    let bytes = serde_json::to_vec_pretty(checkpoint).map_err(|_| RangeFailureKind::LocalIo)?;
    if bytes.is_empty() || bytes.len() as u64 > MAX_CHECKPOINT_BYTES {
        return Err(RangeFailureKind::BudgetExceeded);
    }
    let mut file = OpenOptions::new()
        .create_new(true)
        .write(true)
        .open(&workspace.checkpoint_temp_path)
        .map_err(|_| RangeFailureKind::LocalIo)?;
    file.write_all(&bytes)
        .and_then(|_| file.flush())
        .and_then(|_| file.sync_all())
        .map_err(|_| RangeFailureKind::LocalIo)?;
    drop(file);
    atomic_replace(&workspace.checkpoint_temp_path, &workspace.checkpoint_path)
        .map_err(|_| RangeFailureKind::LocalIo)
}

#[cfg(windows)]
fn atomic_replace(source: &Path, destination: &Path) -> std::io::Result<()> {
    use std::os::windows::ffi::OsStrExt;
    use windows_sys::Win32::Storage::FileSystem::{
        MoveFileExW, MOVEFILE_REPLACE_EXISTING, MOVEFILE_WRITE_THROUGH,
    };

    let source = source
        .as_os_str()
        .encode_wide()
        .chain(std::iter::once(0))
        .collect::<Vec<_>>();
    let destination = destination
        .as_os_str()
        .encode_wide()
        .chain(std::iter::once(0))
        .collect::<Vec<_>>();
    // SAFETY: both buffers are NUL-terminated Windows paths and remain alive for
    // the duration of the synchronous call.
    let status = unsafe {
        MoveFileExW(
            source.as_ptr(),
            destination.as_ptr(),
            MOVEFILE_REPLACE_EXISTING | MOVEFILE_WRITE_THROUGH,
        )
    };
    if status == 0 {
        Err(std::io::Error::last_os_error())
    } else {
        Ok(())
    }
}

#[cfg(not(windows))]
fn atomic_replace(source: &Path, destination: &Path) -> std::io::Result<()> {
    fs::rename(source, destination)
}

fn purge_workspace(workspace: &RangeWorkspace) -> std::io::Result<()> {
    remove_file_if_present(&workspace.part_path)?;
    remove_file_if_present(&workspace.checkpoint_path)?;
    remove_file_if_present(&workspace.checkpoint_temp_path)?;
    if workspace.range_directory.exists() {
        fs::remove_dir_all(&workspace.range_directory)?;
    }
    Ok(())
}

fn remove_file_if_present(path: &Path) -> std::io::Result<()> {
    match fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error),
    }
}

fn regular_file_is_real(path: &Path) -> bool {
    fs::symlink_metadata(path)
        .is_ok_and(|metadata| metadata.file_type().is_file() && !metadata.file_type().is_symlink())
}

fn directory_is_real(path: &Path) -> bool {
    fs::symlink_metadata(path)
        .is_ok_and(|metadata| metadata.file_type().is_dir() && !metadata.file_type().is_symlink())
}

fn plan_sha256(plan: &RangePlan) -> Result<String, RangeFailureKind> {
    serde_json::to_vec(plan)
        .map(|bytes| format!("{:x}", Sha256::digest(bytes)))
        .map_err(|_| RangeFailureKind::RecoveryInvalid)
}

fn file_sha256(path: &Path) -> std::io::Result<(u64, String)> {
    let mut file = File::open(path)?;
    let mut digest = Sha256::new();
    let mut size = 0_u64;
    let mut buffer = [0_u8; 64 * 1024];
    loop {
        let count = file.read(&mut buffer)?;
        if count == 0 {
            break;
        }
        size = size
            .checked_add(count as u64)
            .ok_or_else(|| std::io::Error::other("file length overflow"))?;
        digest.update(&buffer[..count]);
    }
    Ok((size, format!("{:x}", digest.finalize())))
}

fn single_header(headers: &HeaderMap, name: HeaderName) -> Result<&str, RangeFailureKind> {
    let values = headers.get_all(name).iter().collect::<Vec<_>>();
    if values.len() != 1 {
        return Err(RangeFailureKind::Protocol);
    }
    values[0].to_str().map_err(|_| RangeFailureKind::Protocol)
}

fn parse_content_range(value: &str) -> Result<(u64, u64, u64), RangeFailureKind> {
    let value = value
        .strip_prefix("bytes ")
        .ok_or(RangeFailureKind::Protocol)?;
    let (bounds, total) = value.split_once('/').ok_or(RangeFailureKind::Protocol)?;
    let (start, end) = bounds.split_once('-').ok_or(RangeFailureKind::Protocol)?;
    if total == "*" || start.is_empty() || end.is_empty() || total.is_empty() {
        return Err(RangeFailureKind::Protocol);
    }
    Ok((
        parse_ascii_u64(start)?,
        parse_ascii_u64(end)?,
        parse_ascii_u64(total)?,
    ))
}

fn parse_ascii_u64(value: &str) -> Result<u64, RangeFailureKind> {
    if value.is_empty() || !value.bytes().all(|byte| byte.is_ascii_digit()) {
        return Err(RangeFailureKind::Protocol);
    }
    value.parse().map_err(|_| RangeFailureKind::Protocol)
}

fn validate_strong_etag(value: &str) -> Result<(), RangeFailureKind> {
    let bytes = value.as_bytes();
    if bytes.len() < 2
        || bytes.len() > MAX_ETAG_BYTES
        || value.starts_with("W/")
        || bytes.first() != Some(&b'"')
        || bytes.last() != Some(&b'"')
        || bytes[1..bytes.len() - 1]
            .iter()
            .any(|byte| *byte < 0x21 || *byte == b'"' || *byte == 0x7f)
    {
        return Err(RangeFailureKind::InvalidStrongEtag);
    }
    Ok(())
}

fn validate_sha256(value: &str) -> Result<(), RangeFailureKind> {
    if value.len() != 64
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    {
        return Err(RangeFailureKind::InvalidOutputChecksum);
    }
    Ok(())
}

fn failure(kind: RangeFailureKind, checkpoint: &RangeCheckpoint) -> RangeTransferFailure {
    RangeTransferFailure {
        kind,
        ledger: checkpoint.ledger.clone(),
    }
}
