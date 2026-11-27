use serde::de::Error as _;
use serde::{Deserialize, Deserializer, Serialize, Serializer};

use crate::SubsetLabError;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ByteRange {
    start: u64,
    end_exclusive: u64,
}

#[derive(Serialize, Deserialize)]
struct ByteRangeWire {
    start: u64,
    end_exclusive: u64,
}

impl ByteRange {
    pub fn new(start: u64, end_exclusive: u64, source_len: u64) -> Result<Self, SubsetLabError> {
        if start >= end_exclusive || end_exclusive > source_len {
            return Err(SubsetLabError::InvalidRange);
        }
        Ok(Self {
            start,
            end_exclusive,
        })
    }

    pub fn len(self) -> u64 {
        self.end_exclusive - self.start
    }

    pub fn start(self) -> u64 {
        self.start
    }

    pub fn end_exclusive(self) -> u64 {
        self.end_exclusive
    }

    pub fn is_empty(self) -> bool {
        self.start == self.end_exclusive
    }

    /// Builds a half-open byte range covering the final `tail_len` bytes of a source object
    /// (e.g., the 2052-byte Zarr v3 `sharding_indexed` tail directory + CRC32C checksum).
    pub fn tail(tail_len: u64, source_len: u64) -> Result<Self, SubsetLabError> {
        if tail_len == 0 || tail_len > source_len {
            return Err(SubsetLabError::InvalidRange);
        }
        Ok(Self {
            start: source_len - tail_len,
            end_exclusive: source_len,
        })
    }

    /// Converts a Zarr v3 `sharding_indexed` `(offset_u64_le, nbytes_u64_le)` entry into
    /// a validated `ByteRange` strictly preceding the shard's `tail_index_bytes` directory:
    /// - Returns `Ok(None)` for the Zarr v3 empty-chunk sentinel `(u64::MAX, u64::MAX)`;
    /// - Rejects partial `u64::MAX`, zero-length entries (`nbytes == 0`), `u64` addition overflow,
    ///   or any byte range that intrudes into the trailing `tail_index_bytes` region.
    pub fn from_zarr_v3_shard_entry(
        offset: u64,
        nbytes: u64,
        source_len: u64,
        tail_index_bytes: u64,
    ) -> Result<Option<Self>, SubsetLabError> {
        if offset == u64::MAX && nbytes == u64::MAX {
            return Ok(None);
        }
        if offset == u64::MAX || nbytes == u64::MAX || nbytes == 0 || tail_index_bytes > source_len
        {
            return Err(SubsetLabError::InvalidRange);
        }
        let payload_limit = source_len - tail_index_bytes;
        let end_exclusive = offset
            .checked_add(nbytes)
            .ok_or(SubsetLabError::InvalidRange)?;
        if end_exclusive > payload_limit {
            return Err(SubsetLabError::InvalidRange);
        }
        Ok(Some(Self {
            start: offset,
            end_exclusive,
        }))
    }
}

/// Exact byte length of a Zarr v3 `sharding_indexed` tail index for a `[1, 2, 8, 8]`
/// (128 inner chunks) shard grid: `128 * 16` bytes (`[offset_u64_le, nbytes_u64_le]`)
/// plus a 4-byte `crc32c` checksum (`2048 + 4 = 2052` bytes).
pub const ZARR_V3_SHARD_128_TAIL_INDEX_BYTES: u64 = 2_052;

impl Serialize for ByteRange {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        ByteRangeWire {
            start: self.start,
            end_exclusive: self.end_exclusive,
        }
        .serialize(serializer)
    }
}

impl<'de> Deserialize<'de> for ByteRange {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let wire = ByteRangeWire::deserialize(deserializer)?;
        if wire.start >= wire.end_exclusive {
            return Err(D::Error::custom("invalid half-open byte range"));
        }
        Ok(Self {
            start: wire.start,
            end_exclusive: wire.end_exclusive,
        })
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MergePolicy {
    pub maximum_gap_bytes: u64,
    pub maximum_extra_bytes: u64,
    pub maximum_ranges: usize,
    pub maximum_transfer_bytes: u64,
}

impl MergePolicy {
    /// Zero-gap coalescing policy (`maximum_gap_bytes = 0`, `maximum_extra_bytes = 0`)
    /// for Zarr v3 sharded containers (`sharding_indexed`) and strided inner chunks
    /// (such as OlmoEarth v1.3 `[2, 8, 8]` inner chunks under Matryoshka `64D` prefix slicing).
    ///
    /// Strictly adjacent or overlapping byte ranges (`gap == 0`) are coalesced into one
    /// Range request, while any positive gap (`gap > 0`) across unselected inner chunks
    /// is never bridged with wasted transfer bytes.
    pub const fn zero_gap() -> Self {
        Self {
            maximum_gap_bytes: 0,
            maximum_extra_bytes: 0,
            maximum_ranges: 4_096,
            maximum_transfer_bytes: 512 * 1024 * 1024,
        }
    }

    /// Returns a copy of this policy with `maximum_gap_bytes` and `maximum_extra_bytes`
    /// set to `0` so non-adjacent inner chunks are never coalesced across unselected gaps.
    pub const fn with_zero_gap(self) -> Self {
        Self {
            maximum_gap_bytes: 0,
            maximum_extra_bytes: 0,
            ..self
        }
    }
}

impl Default for MergePolicy {
    fn default() -> Self {
        Self {
            maximum_gap_bytes: 16 * 1024 * 1024,
            maximum_extra_bytes: 64 * 1024 * 1024,
            maximum_ranges: 4_096,
            maximum_transfer_bytes: 512 * 1024 * 1024,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct RangePlan {
    source_len: u64,
    ranges: Vec<ByteRange>,
    selected_bytes: u64,
    transfer_bytes: u64,
    extra_bytes: u64,
}

#[derive(Deserialize)]
struct RangePlanWire {
    source_len: u64,
    ranges: Vec<ByteRange>,
    selected_bytes: u64,
    transfer_bytes: u64,
    extra_bytes: u64,
}

impl<'de> Deserialize<'de> for RangePlan {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let wire = RangePlanWire::deserialize(deserializer)?;
        let plan = Self {
            source_len: wire.source_len,
            ranges: wire.ranges,
            selected_bytes: wire.selected_bytes,
            transfer_bytes: wire.transfer_bytes,
            extra_bytes: wire.extra_bytes,
        };
        plan.validate().map_err(D::Error::custom)?;
        Ok(plan)
    }
}

impl RangePlan {
    pub fn build(
        source_len: u64,
        ranges: impl IntoIterator<Item = ByteRange>,
        policy: MergePolicy,
    ) -> Result<Self, SubsetLabError> {
        if source_len == 0 || policy.maximum_ranges == 0 {
            return Err(SubsetLabError::RangeBudgetExceeded);
        }
        let mut ranges = ranges.into_iter().collect::<Vec<_>>();
        if ranges.is_empty()
            || ranges
                .iter()
                .any(|range| range.start >= range.end_exclusive || range.end_exclusive > source_len)
        {
            return Err(SubsetLabError::InvalidRange);
        }
        ranges.sort_unstable_by_key(|range| (range.start, range.end_exclusive));

        let mut selected = Vec::<ByteRange>::with_capacity(ranges.len());
        for range in ranges {
            if let Some(previous) = selected.last_mut() {
                if range.start <= previous.end_exclusive {
                    previous.end_exclusive = previous.end_exclusive.max(range.end_exclusive);
                    continue;
                }
            }
            selected.push(range);
        }
        let selected_bytes = checked_sum(selected.iter().map(|range| range.len()))?;

        let mut merged = Vec::<ByteRange>::with_capacity(selected.len());
        let mut extra_bytes = 0_u64;
        for range in selected {
            if let Some(previous) = merged.last_mut() {
                let gap = range.start - previous.end_exclusive;
                let candidate_extra = extra_bytes.checked_add(gap);
                if gap <= policy.maximum_gap_bytes
                    && candidate_extra.is_some_and(|value| {
                        value <= policy.maximum_extra_bytes
                            && selected_bytes
                                .checked_add(value)
                                .is_some_and(|bytes| bytes <= policy.maximum_transfer_bytes)
                    })
                {
                    previous.end_exclusive = range.end_exclusive;
                    extra_bytes = candidate_extra.expect("checked by merge condition");
                    continue;
                }
            }
            merged.push(range);
        }

        let transfer_bytes = checked_sum(merged.iter().map(|range| range.len()))?;
        if merged.len() > policy.maximum_ranges
            || transfer_bytes > policy.maximum_transfer_bytes
            || transfer_bytes != selected_bytes.saturating_add(extra_bytes)
        {
            return Err(SubsetLabError::RangeBudgetExceeded);
        }
        Ok(Self {
            source_len,
            ranges: merged,
            selected_bytes,
            transfer_bytes,
            extra_bytes,
        })
    }

    pub fn validate(&self) -> Result<(), SubsetLabError> {
        if self.source_len == 0
            || self.ranges.is_empty()
            || self.ranges.iter().any(|range| {
                range.start >= range.end_exclusive || range.end_exclusive > self.source_len
            })
            || self
                .ranges
                .windows(2)
                .any(|pair| pair[0].end_exclusive >= pair[1].start)
        {
            return Err(SubsetLabError::InvalidRange);
        }
        let transfer_bytes = checked_sum(self.ranges.iter().map(|range| range.len()))?;
        if self.selected_bytes == 0
            || transfer_bytes != self.transfer_bytes
            || self.selected_bytes > self.transfer_bytes
            || self.extra_bytes != self.transfer_bytes - self.selected_bytes
        {
            return Err(SubsetLabError::RangeBudgetExceeded);
        }
        Ok(())
    }

    pub fn source_len(&self) -> u64 {
        self.source_len
    }

    pub fn ranges(&self) -> &[ByteRange] {
        &self.ranges
    }

    pub fn selected_bytes(&self) -> u64 {
        self.selected_bytes
    }

    pub fn transfer_bytes(&self) -> u64 {
        self.transfer_bytes
    }

    pub fn extra_bytes(&self) -> u64 {
        self.extra_bytes
    }
}

fn checked_sum(mut values: impl Iterator<Item = u64>) -> Result<u64, SubsetLabError> {
    values.try_fold(0_u64, |sum, value| {
        sum.checked_add(value)
            .ok_or(SubsetLabError::RangeBudgetExceeded)
    })
}
