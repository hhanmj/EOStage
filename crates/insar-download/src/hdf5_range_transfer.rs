//! Offline-only physical Range planning for client-derived NISAR HDF5 subsets.
//!
//! The runtime adapter added in this module is deliberately separate from the
//! complete-product route. Unsupported Range behavior is terminal and never
//! falls back to a full-file transfer.
#![cfg_attr(not(test), allow(dead_code))]

use std::collections::{BTreeMap, BTreeSet};
#[cfg(feature = "nisar-subset-production")]
use std::ffi::CString;
use std::fs::{self, File, OpenOptions, TryLockError};
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::sync::Arc;
#[cfg(feature = "nisar-subset-production")]
use std::sync::Mutex;

use hdf5::filters::Filter;
use hdf5::types::{CompoundField, CompoundType, FloatSize, TypeDescriptor};
use hdf5::{Dataset, Datatype};
use hdf5_sys::h5d::H5D_fill_value_t::{
    H5D_FILL_VALUE_DEFAULT, H5D_FILL_VALUE_ERROR, H5D_FILL_VALUE_UNDEFINED,
    H5D_FILL_VALUE_USER_DEFINED,
};
#[cfg(feature = "nisar-subset-production")]
use hdf5_sys::h5p::H5P_DEFAULT;
use hdf5_sys::h5p::{H5Pfill_value_defined, H5Pget_fill_value};
use hdf5_sys::h5t::H5Tencode;
#[cfg(feature = "nisar-subset-production")]
use hdf5_sys::h5t::{H5Tclose, H5Topen2};
use insar_core::{
    validate_request_against_snapshot_item, NisarSubsetByteRangeV1, NisarSubsetChecksumAlgorithm,
    NisarSubsetFrequency, NisarSubsetRangeState, NisarSubsetRequestManifestV1,
    NisarSubsetSourceIdentityV1, NisarSubsetSourceValidatorV1, NisarSubsetTransferManifestV1,
    NisarSubsetTransferState, NisarSubsetValidatorKind, ProxyPolicy, SnapshotItem,
    NISAR_SUBSET_MAX_TRANSFER_CANONICAL_JSON_BYTES,
};
use reqwest::header::{HeaderName, CONTENT_ENCODING, CONTENT_LENGTH, CONTENT_RANGE, ETAG};
use reqwest::StatusCode;
use serde::{Deserialize, Serialize};
#[cfg(feature = "nisar-subset-production")]
use serde_json::{Map, Value};
use sha2::{Digest, Sha256};
use thiserror::Error;

#[cfg(feature = "nisar-subset-production")]
use insar_provider::asf::nisar_subset::{plan_nisar_subset, supported_dataset_polarizations};

use crate::http_transfer::ExactRangeSendOutcome;
#[cfg(feature = "nisar-subset-production")]
use crate::nisar_science_root::NisarScienceRoot;
use crate::pinned_directory::PinnedDirectory;
use crate::safe_promotion::{promote_verified_closed_artifact, VerifiedClosedArtifact};
use crate::{DestinationLeaseController, DestinationLeaseError, HttpTransfer, ResolvedAsset};

pub(crate) const HDF5_RANGE_MAX_COUNT: usize = 4_096;
// A fixed 512 MiB physical-plan ceiling keeps the subset capability bounded
// while accommodating legitimate scene/AOI combinations whose compressed
// chunks exceed the original first-scene-derived 256 MiB ceiling. The live
// network budget independently applies the stricter 10% source-size ratio.
pub(crate) const HDF5_RANGE_MAX_REMOTE_BYTES: u64 = 512 * 1024 * 1024;
// Keep requests large enough that a bounded plan plus one approved redirect
// per Range can fit under the 256-request live cap. Response streaming still
// uses the separate 64 KiB read buffer below, so this does not create a
// multi-megabyte stack buffer.
pub(crate) const HDF5_RANGE_NETWORK_SLICE_BYTES: u64 = 4 * 1024 * 1024;
const HDF5_RANGE_READ_BUFFER_BYTES: usize = 64 * 1024;
const SCIENCE_ALLOCATED_RANGE_MAX_COUNT_V2: usize = HDF5_RANGE_MAX_COUNT;

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub(crate) struct NisarScienceAllocatedRangeEvidenceV2 {
    intervals: Vec<(u64, u64)>,
}

#[derive(Clone, Copy, Debug, Error, PartialEq, Eq)]
pub(crate) enum NisarScienceAllocationEvidenceErrorV2 {
    #[error("science allocated-range evidence is invalid")]
    RecoveryInvalid,
    #[error("exact allocated-range evidence is unsupported on this filesystem")]
    AllocationEvidenceUnsupported,
}

impl NisarScienceAllocatedRangeEvidenceV2 {
    pub(crate) fn covers_exactly(&self, start: u64, end_exclusive: u64) -> bool {
        if start >= end_exclusive || self.intervals.is_empty() {
            return false;
        }
        let mut cursor = start;
        for &(range_start, range_end) in &self.intervals {
            if range_start != cursor || range_start >= range_end || range_end > end_exclusive {
                return false;
            }
            cursor = range_end;
        }
        cursor == end_exclusive
    }
}

fn normalize_science_allocated_ranges_v2(
    start: u64,
    end_exclusive: u64,
    ranges: impl IntoIterator<Item = (u64, u64)>,
) -> Result<NisarScienceAllocatedRangeEvidenceV2, NisarScienceAllocationEvidenceErrorV2> {
    use NisarScienceAllocationEvidenceErrorV2 as Hdf5RangeTransferError;
    if start >= end_exclusive {
        return Err(Hdf5RangeTransferError::RecoveryInvalid);
    }
    let mut intervals = Vec::new();
    for (range_start, range_end) in ranges {
        let clipped_start = range_start.max(start);
        let clipped_end = range_end.min(end_exclusive);
        if clipped_start >= clipped_end {
            continue;
        }
        if let Some(last) = intervals.last_mut() {
            let last: &mut (u64, u64) = last;
            if clipped_start < last.1 {
                return Err(Hdf5RangeTransferError::RecoveryInvalid);
            }
            if clipped_start == last.1 {
                last.1 = clipped_end;
                continue;
            }
        }
        if intervals.len() >= SCIENCE_ALLOCATED_RANGE_MAX_COUNT_V2 {
            return Err(Hdf5RangeTransferError::RecoveryInvalid);
        }
        intervals.push((clipped_start, clipped_end));
    }
    Ok(NisarScienceAllocatedRangeEvidenceV2 { intervals })
}

#[cfg(windows)]
pub(crate) fn science_allocated_range_evidence_v2(
    file: &mut File,
    start: u64,
    end_exclusive: u64,
) -> Result<NisarScienceAllocatedRangeEvidenceV2, NisarScienceAllocationEvidenceErrorV2> {
    use std::mem::{size_of, size_of_val};
    use std::os::windows::io::AsRawHandle;
    use windows_sys::Win32::System::Ioctl::{
        FILE_ALLOCATED_RANGE_BUFFER, FSCTL_QUERY_ALLOCATED_RANGES,
    };
    use windows_sys::Win32::System::IO::DeviceIoControl;
    use NisarScienceAllocationEvidenceErrorV2 as Hdf5RangeTransferError;

    if start >= end_exclusive
        || end_exclusive
            > file
                .metadata()
                .map_err(|_| Hdf5RangeTransferError::RecoveryInvalid)?
                .len()
    {
        return Err(Hdf5RangeTransferError::RecoveryInvalid);
    }
    let input = FILE_ALLOCATED_RANGE_BUFFER {
        FileOffset: i64::try_from(start).map_err(|_| Hdf5RangeTransferError::RecoveryInvalid)?,
        Length: i64::try_from(end_exclusive - start)
            .map_err(|_| Hdf5RangeTransferError::RecoveryInvalid)?,
    };
    let mut output = [FILE_ALLOCATED_RANGE_BUFFER::default(); SCIENCE_ALLOCATED_RANGE_MAX_COUNT_V2];
    let mut returned = 0_u32;
    let succeeded = unsafe {
        DeviceIoControl(
            file.as_raw_handle(),
            FSCTL_QUERY_ALLOCATED_RANGES,
            (&raw const input).cast(),
            u32::try_from(size_of::<FILE_ALLOCATED_RANGE_BUFFER>())
                .map_err(|_| Hdf5RangeTransferError::RecoveryInvalid)?,
            output.as_mut_ptr().cast(),
            u32::try_from(size_of_val(&output))
                .map_err(|_| Hdf5RangeTransferError::RecoveryInvalid)?,
            &mut returned,
            std::ptr::null_mut(),
        )
    };
    if succeeded == 0 {
        return Err(Hdf5RangeTransferError::AllocationEvidenceUnsupported);
    }
    if !(returned as usize).is_multiple_of(size_of::<FILE_ALLOCATED_RANGE_BUFFER>())
        || returned as usize > size_of_val(&output)
    {
        return Err(Hdf5RangeTransferError::RecoveryInvalid);
    }
    let count = returned as usize / size_of::<FILE_ALLOCATED_RANGE_BUFFER>();
    let ranges = output[..count]
        .iter()
        .map(|range| {
            let range_start = u64::try_from(range.FileOffset)
                .map_err(|_| Hdf5RangeTransferError::RecoveryInvalid)?;
            let length =
                u64::try_from(range.Length).map_err(|_| Hdf5RangeTransferError::RecoveryInvalid)?;
            let range_end = range_start
                .checked_add(length)
                .ok_or(Hdf5RangeTransferError::RecoveryInvalid)?;
            Ok((range_start, range_end))
        })
        .collect::<Result<Vec<_>, Hdf5RangeTransferError>>()?;
    normalize_science_allocated_ranges_v2(start, end_exclusive, ranges)
}

const LINUX_FIEMAP_EXTENT_LAST_V2: u32 = 0x0000_0001;
const LINUX_FIEMAP_EXTENT_SHARED_V2: u32 = 0x0000_2000;
const LINUX_FIEMAP_EXTENT_COARSE_FLAGS_V2: u32 = 0x0000_0002
    | 0x0000_0004
    | 0x0000_0008
    | 0x0000_0080
    | 0x0000_0100
    | 0x0000_0200
    | 0x0000_0400
    | 0x0000_0800
    | 0x0000_1000;

fn validate_linux_fiemap_extent_flags_v2(
    flags: u32,
) -> Result<(), NisarScienceAllocationEvidenceErrorV2> {
    use NisarScienceAllocationEvidenceErrorV2 as Hdf5RangeTransferError;
    let known_exact = LINUX_FIEMAP_EXTENT_LAST_V2 | LINUX_FIEMAP_EXTENT_SHARED_V2;
    if flags & LINUX_FIEMAP_EXTENT_COARSE_FLAGS_V2 != 0 || flags & !known_exact != 0 {
        return Err(Hdf5RangeTransferError::AllocationEvidenceUnsupported);
    }
    Ok(())
}

#[cfg(target_os = "linux")]
pub(crate) fn science_allocated_range_evidence_v2(
    file: &mut File,
    start: u64,
    end_exclusive: u64,
) -> Result<NisarScienceAllocatedRangeEvidenceV2, NisarScienceAllocationEvidenceErrorV2> {
    use std::mem::size_of;
    use std::os::fd::AsRawFd;
    use NisarScienceAllocationEvidenceErrorV2 as Hdf5RangeTransferError;

    #[repr(C)]
    #[derive(Clone, Copy)]
    struct LinuxFiemapHeaderV2 {
        start: u64,
        length: u64,
        flags: u32,
        mapped_extents: u32,
        extent_count: u32,
        reserved: u32,
    }

    #[repr(C)]
    #[derive(Clone, Copy)]
    struct LinuxFiemapExtentV2 {
        logical: u64,
        physical: u64,
        length: u64,
        reserved64: [u64; 2],
        flags: u32,
        reserved: [u32; 3],
    }

    const FS_IOC_FIEMAP_V2: libc::c_ulong = 0xc020_660b;
    const FIEMAP_FLAG_SYNC_V2: u32 = 0x0000_0001;

    if size_of::<LinuxFiemapHeaderV2>() != 32 || size_of::<LinuxFiemapExtentV2>() != 56 {
        return Err(Hdf5RangeTransferError::AllocationEvidenceUnsupported);
    }
    if start >= end_exclusive
        || end_exclusive
            > file
                .metadata()
                .map_err(|_| Hdf5RangeTransferError::RecoveryInvalid)?
                .len()
    {
        return Err(Hdf5RangeTransferError::RecoveryInvalid);
    }
    let allocation_bytes = size_of::<LinuxFiemapHeaderV2>()
        .checked_add(
            size_of::<LinuxFiemapExtentV2>()
                .checked_mul(SCIENCE_ALLOCATED_RANGE_MAX_COUNT_V2)
                .ok_or(Hdf5RangeTransferError::AllocationEvidenceUnsupported)?,
        )
        .ok_or(Hdf5RangeTransferError::AllocationEvidenceUnsupported)?;
    let word_count = allocation_bytes
        .checked_add(size_of::<u64>() - 1)
        .and_then(|bytes| bytes.checked_div(size_of::<u64>()))
        .ok_or(Hdf5RangeTransferError::AllocationEvidenceUnsupported)?;
    let mut storage = vec![0_u64; word_count];
    let header_ptr = storage.as_mut_ptr().cast::<LinuxFiemapHeaderV2>();
    unsafe {
        header_ptr.write(LinuxFiemapHeaderV2 {
            start,
            length: end_exclusive - start,
            flags: FIEMAP_FLAG_SYNC_V2,
            mapped_extents: 0,
            extent_count: u32::try_from(SCIENCE_ALLOCATED_RANGE_MAX_COUNT_V2)
                .map_err(|_| Hdf5RangeTransferError::AllocationEvidenceUnsupported)?,
            reserved: 0,
        });
    }
    let result = unsafe {
        libc::ioctl(
            file.as_raw_fd(),
            FS_IOC_FIEMAP_V2,
            storage.as_mut_ptr().cast::<libc::c_void>(),
        )
    };
    if result != 0 {
        return Err(Hdf5RangeTransferError::AllocationEvidenceUnsupported);
    }
    let header = unsafe { &*header_ptr };
    let mapped = usize::try_from(header.mapped_extents)
        .map_err(|_| Hdf5RangeTransferError::AllocationEvidenceUnsupported)?;
    if mapped > SCIENCE_ALLOCATED_RANGE_MAX_COUNT_V2
        || mapped == SCIENCE_ALLOCATED_RANGE_MAX_COUNT_V2
        || header.extent_count as usize != SCIENCE_ALLOCATED_RANGE_MAX_COUNT_V2
    {
        return Err(Hdf5RangeTransferError::AllocationEvidenceUnsupported);
    }
    let extents_ptr = unsafe {
        storage
            .as_ptr()
            .cast::<u8>()
            .add(size_of::<LinuxFiemapHeaderV2>())
            .cast::<LinuxFiemapExtentV2>()
    };
    let extents = unsafe { std::slice::from_raw_parts(extents_ptr, mapped) };
    let mut ranges = Vec::with_capacity(mapped);
    for extent in extents {
        validate_linux_fiemap_extent_flags_v2(extent.flags)?;
        if extent.length == 0 {
            return Err(Hdf5RangeTransferError::RecoveryInvalid);
        }
        let end = extent
            .logical
            .checked_add(extent.length)
            .ok_or(Hdf5RangeTransferError::RecoveryInvalid)?;
        ranges.push((extent.logical, end));
    }
    normalize_science_allocated_ranges_v2(start, end_exclusive, ranges)
}

#[cfg(target_os = "macos")]
pub(crate) fn science_allocated_range_evidence_v2(
    file: &mut File,
    start: u64,
    end_exclusive: u64,
) -> Result<NisarScienceAllocatedRangeEvidenceV2, NisarScienceAllocationEvidenceErrorV2> {
    use std::os::fd::AsRawFd;
    use NisarScienceAllocationEvidenceErrorV2 as Hdf5RangeTransferError;

    if start >= end_exclusive
        || end_exclusive
            > file
                .metadata()
                .map_err(|_| Hdf5RangeTransferError::RecoveryInvalid)?
                .len()
    {
        return Err(Hdf5RangeTransferError::RecoveryInvalid);
    }
    let saved_position = file
        .stream_position()
        .map_err(|_| Hdf5RangeTransferError::RecoveryInvalid)?;
    let fd = file.as_raw_fd();
    let result = (|| {
        let mut ranges = Vec::new();
        let mut cursor = start;
        while cursor < end_exclusive {
            let cursor_off =
                i64::try_from(cursor).map_err(|_| Hdf5RangeTransferError::RecoveryInvalid)?;
            let data_off = unsafe { libc::lseek(fd, cursor_off, libc::SEEK_DATA) };
            if data_off < 0 {
                return match std::io::Error::last_os_error().raw_os_error() {
                    Some(libc::ENXIO) => break,
                    Some(libc::ENOTSUP | libc::EINVAL | libc::ENOSYS) => {
                        Err(Hdf5RangeTransferError::AllocationEvidenceUnsupported)
                    }
                    _ => Err(Hdf5RangeTransferError::RecoveryInvalid),
                };
            }
            let data_start =
                u64::try_from(data_off).map_err(|_| Hdf5RangeTransferError::RecoveryInvalid)?;
            if data_start >= end_exclusive {
                break;
            }
            let hole_off = unsafe { libc::lseek(fd, data_off, libc::SEEK_HOLE) };
            if hole_off < 0 {
                return match std::io::Error::last_os_error().raw_os_error() {
                    Some(libc::ENOTSUP | libc::EINVAL | libc::ENOSYS) => {
                        Err(Hdf5RangeTransferError::AllocationEvidenceUnsupported)
                    }
                    _ => Err(Hdf5RangeTransferError::RecoveryInvalid),
                };
            }
            let hole_start =
                u64::try_from(hole_off).map_err(|_| Hdf5RangeTransferError::RecoveryInvalid)?;
            if hole_start <= data_start {
                return Err(Hdf5RangeTransferError::RecoveryInvalid);
            }
            if ranges.len() >= SCIENCE_ALLOCATED_RANGE_MAX_COUNT_V2 {
                return Err(Hdf5RangeTransferError::AllocationEvidenceUnsupported);
            }
            ranges.push((data_start.max(start), hole_start.min(end_exclusive)));
            cursor = hole_start;
        }
        normalize_science_allocated_ranges_v2(start, end_exclusive, ranges)
    })();
    let _ = file.seek(SeekFrom::Start(saved_position));
    result
}

#[cfg(not(any(windows, target_os = "linux", target_os = "macos")))]
pub(crate) fn science_allocated_range_evidence_v2(
    _file: &mut File,
    _start: u64,
    _end_exclusive: u64,
) -> Result<NisarScienceAllocatedRangeEvidenceV2, NisarScienceAllocationEvidenceErrorV2> {
    use NisarScienceAllocationEvidenceErrorV2 as Hdf5RangeTransferError;
    Err(Hdf5RangeTransferError::AllocationEvidenceUnsupported)
}

#[cfg(test)]
pub(crate) fn coarse_science_allocation_evidence_v2_for_test(
) -> Result<(), NisarScienceAllocationEvidenceErrorV2> {
    validate_linux_fiemap_extent_flags_v2(0x0000_1000)
}

pub(crate) fn validate_science_allocated_range_evidence_v2(
    file: &mut File,
    start: u64,
    end_exclusive: u64,
    expected: &NisarScienceAllocatedRangeEvidenceV2,
) -> Result<(), NisarScienceAllocationEvidenceErrorV2> {
    use NisarScienceAllocationEvidenceErrorV2 as Hdf5RangeTransferError;
    if !expected.covers_exactly(start, end_exclusive)
        || science_allocated_range_evidence_v2(file, start, end_exclusive)? != *expected
    {
        return Err(Hdf5RangeTransferError::RecoveryInvalid);
    }
    Ok(())
}

#[cfg(all(test, windows))]
pub(crate) fn punch_science_hole_v2_for_test(
    file: &mut File,
    start: u64,
    end_exclusive: u64,
) -> Result<(), Hdf5RangeTransferError> {
    use std::mem::size_of;
    use std::os::windows::io::AsRawHandle;
    use windows_sys::Win32::System::Ioctl::{
        FILE_ZERO_DATA_INFORMATION, FSCTL_SET_SPARSE, FSCTL_SET_ZERO_DATA,
    };
    use windows_sys::Win32::System::IO::DeviceIoControl;

    if start >= end_exclusive
        || end_exclusive
            > file
                .metadata()
                .map_err(|_| Hdf5RangeTransferError::RecoveryInvalid)?
                .len()
    {
        return Err(Hdf5RangeTransferError::RecoveryInvalid);
    }
    let mut returned = 0_u32;
    let sparse = unsafe {
        DeviceIoControl(
            file.as_raw_handle(),
            FSCTL_SET_SPARSE,
            std::ptr::null(),
            0,
            std::ptr::null_mut(),
            0,
            &mut returned,
            std::ptr::null_mut(),
        )
    };
    if sparse == 0 {
        return Err(Hdf5RangeTransferError::RecoveryInvalid);
    }
    let zero = FILE_ZERO_DATA_INFORMATION {
        FileOffset: i64::try_from(start).map_err(|_| Hdf5RangeTransferError::RecoveryInvalid)?,
        BeyondFinalZero: i64::try_from(end_exclusive)
            .map_err(|_| Hdf5RangeTransferError::RecoveryInvalid)?,
    };
    let zeroed = unsafe {
        DeviceIoControl(
            file.as_raw_handle(),
            FSCTL_SET_ZERO_DATA,
            (&raw const zero).cast(),
            u32::try_from(size_of::<FILE_ZERO_DATA_INFORMATION>())
                .map_err(|_| Hdf5RangeTransferError::RecoveryInvalid)?,
            std::ptr::null_mut(),
            0,
            &mut returned,
            std::ptr::null_mut(),
        )
    };
    if zeroed == 0 {
        return Err(Hdf5RangeTransferError::RecoveryInvalid);
    }
    Ok(())
}

#[cfg(all(test, target_os = "linux"))]
pub(crate) fn punch_science_hole_v2_for_test(
    file: &mut File,
    start: u64,
    end_exclusive: u64,
) -> Result<(), Hdf5RangeTransferError> {
    use std::os::fd::AsRawFd;

    if start >= end_exclusive
        || end_exclusive
            > file
                .metadata()
                .map_err(|_| Hdf5RangeTransferError::RecoveryInvalid)?
                .len()
    {
        return Err(Hdf5RangeTransferError::RecoveryInvalid);
    }
    let result = unsafe {
        libc::fallocate(
            file.as_raw_fd(),
            libc::FALLOC_FL_PUNCH_HOLE | libc::FALLOC_FL_KEEP_SIZE,
            i64::try_from(start).map_err(|_| Hdf5RangeTransferError::RecoveryInvalid)?,
            i64::try_from(end_exclusive - start)
                .map_err(|_| Hdf5RangeTransferError::RecoveryInvalid)?,
        )
    };
    if result != 0 {
        return Err(Hdf5RangeTransferError::RecoveryInvalid);
    }
    Ok(())
}

#[cfg(all(test, target_os = "macos"))]
pub(crate) fn punch_science_hole_v2_for_test(
    file: &mut File,
    start: u64,
    end_exclusive: u64,
) -> Result<(), Hdf5RangeTransferError> {
    use std::os::fd::AsRawFd;

    if start >= end_exclusive
        || end_exclusive
            > file
                .metadata()
                .map_err(|_| Hdf5RangeTransferError::RecoveryInvalid)?
                .len()
    {
        return Err(Hdf5RangeTransferError::RecoveryInvalid);
    }
    file.sync_all()
        .map_err(|_| Hdf5RangeTransferError::RecoveryInvalid)?;
    let punch = libc::fpunchhole_t {
        fp_flags: 0,
        reserved: 0,
        fp_offset: i64::try_from(start).map_err(|_| Hdf5RangeTransferError::RecoveryInvalid)?,
        fp_length: i64::try_from(end_exclusive - start)
            .map_err(|_| Hdf5RangeTransferError::RecoveryInvalid)?,
    };
    let result = unsafe { libc::fcntl(file.as_raw_fd(), libc::F_PUNCHHOLE, &punch) };
    if result != 0 {
        return Err(Hdf5RangeTransferError::RecoveryInvalid);
    }
    Ok(())
}

#[cfg(all(test, unix, not(any(target_os = "linux", target_os = "macos"))))]
pub(crate) fn punch_science_hole_v2_for_test(
    _file: &mut File,
    _start: u64,
    _end_exclusive: u64,
) -> Result<(), Hdf5RangeTransferError> {
    Err(Hdf5RangeTransferError::RecoveryInvalid)
}

#[cfg(all(test, not(any(windows, unix))))]
pub(crate) fn punch_science_hole_v2_for_test(
    _file: &mut File,
    _start: u64,
    _end_exclusive: u64,
) -> Result<(), Hdf5RangeTransferError> {
    Err(Hdf5RangeTransferError::RecoveryInvalid)
}

#[cfg(feature = "nisar-subset-production")]
pub const NISAR_LIVE_METADATA_PREFIX_BYTES: u64 = 195_035_136;
#[cfg(feature = "nisar-subset-production")]
pub const NISAR_LIVE_MAX_REQUESTS: u64 = 256;
#[cfg(feature = "nisar-subset-production")]
pub const NISAR_LIVE_MAX_RESPONSE_BODY_BYTES: u64 = 512 * 1024 * 1024;

#[cfg(feature = "nisar-subset-production")]
// The independently validated C2 single-frequency scene needs 8.82% of the
// source because the fixed AOI spans 276 compressed science chunks. Keep a
// small fixed margin while the 512 MiB absolute ceiling remains authoritative.
const NISAR_LIVE_MAX_RESPONSE_RATIO_NUMERATOR: u64 = 10;
#[cfg(feature = "nisar-subset-production")]
const NISAR_LIVE_MAX_RESPONSE_RATIO_DENOMINATOR: u64 = 100;

#[cfg(feature = "nisar-subset-production")]
#[derive(Clone, Debug)]
#[doc(hidden)]
pub struct NisarLiveNetworkBudgetV1 {
    state: Arc<Mutex<NisarLiveNetworkBudgetState>>,
    max_response_body_bytes: u64,
}

#[cfg(feature = "nisar-subset-production")]
#[derive(Debug, Default)]
struct NisarLiveNetworkBudgetState {
    reserved_requests: u64,
    reserved_response_body_bytes: u64,
}

#[cfg(feature = "nisar-subset-production")]
pub(crate) fn nisar_live_budget_response_limit(
    source_size: u64,
) -> Result<u64, Hdf5RangeTransferError> {
    if source_size == 0 {
        return Err(Hdf5RangeTransferError::RangeBytesExceeded);
    }
    let ratio_limit = source_size
        .checked_mul(NISAR_LIVE_MAX_RESPONSE_RATIO_NUMERATOR)
        .and_then(|value| value.checked_div(NISAR_LIVE_MAX_RESPONSE_RATIO_DENOMINATOR))
        .ok_or(Hdf5RangeTransferError::RangeBytesExceeded)?;
    Ok(NISAR_LIVE_MAX_RESPONSE_BODY_BYTES.min(ratio_limit))
}

#[cfg(feature = "nisar-subset-production")]
impl NisarLiveNetworkBudgetV1 {
    pub(crate) fn new(source_size: u64) -> Result<Self, Hdf5RangeTransferError> {
        Ok(Self {
            state: Arc::new(Mutex::new(NisarLiveNetworkBudgetState::default())),
            max_response_body_bytes: nisar_live_budget_response_limit(source_size)?,
        })
    }

    pub(crate) fn reserve_request(&self) -> Result<(), Hdf5RangeTransferError> {
        let mut state = self
            .state
            .lock()
            .map_err(|_| Hdf5RangeTransferError::RangeCountExceeded)?;
        let next = state
            .reserved_requests
            .checked_add(1)
            .ok_or(Hdf5RangeTransferError::RangeCountExceeded)?;
        if next > NISAR_LIVE_MAX_REQUESTS {
            return Err(Hdf5RangeTransferError::RangeCountExceeded);
        }
        state.reserved_requests = next;
        Ok(())
    }

    pub(crate) fn reserve_response_body_bytes(
        &self,
        bytes: u64,
    ) -> Result<(), Hdf5RangeTransferError> {
        let mut state = self
            .state
            .lock()
            .map_err(|_| Hdf5RangeTransferError::RangeBytesExceeded)?;
        let next = state
            .reserved_response_body_bytes
            .checked_add(bytes)
            .ok_or(Hdf5RangeTransferError::RangeBytesExceeded)?;
        if next > self.max_response_body_bytes {
            return Err(Hdf5RangeTransferError::RangeBytesExceeded);
        }
        state.reserved_response_body_bytes = next;
        Ok(())
    }

    pub(crate) fn validate_actual(
        &self,
        request_count: u64,
        response_body_bytes: u64,
    ) -> Result<(), Hdf5RangeTransferError> {
        validate_nisar_live_acceptance_budget(request_count, response_body_bytes)?;
        if response_body_bytes > self.max_response_body_bytes {
            return Err(Hdf5RangeTransferError::RangeBytesExceeded);
        }
        let state = self
            .state
            .lock()
            .map_err(|_| Hdf5RangeTransferError::RangeBytesExceeded)?;
        if request_count > state.reserved_requests
            || response_body_bytes > state.reserved_response_body_bytes
        {
            return Err(Hdf5RangeTransferError::RecoveryInvalid);
        }
        Ok(())
    }

    /// Validate the nominal science transfer against the budget already
    /// reserved by preflight, metadata prefix, and metadata discovery.
    ///
    /// The transport layer remains authoritative for retries and actual
    /// response sizes.  This check is an explicit admission-time lower bound
    /// so a plan which cannot fit even without retries is rejected before the
    /// first science Range is sent.
    pub(crate) fn validate_planned_science(
        &self,
        plan: &Hdf5RangePlanV1,
    ) -> Result<(), Hdf5RangeTransferError> {
        let mut request_count = 0_u64;
        let mut response_body_bytes = 0_u64;
        for (index, range) in plan.ranges().iter().enumerate() {
            if plan.metadata_prefix_bytes() > 0
                && index == 0
                && range.start == 0
                && range.end_exclusive == plan.metadata_prefix_bytes()
            {
                continue;
            }
            let bytes = range
                .end_exclusive
                .checked_sub(range.start)
                .ok_or(Hdf5RangeTransferError::RangeInvalid)?;
            let slices = bytes
                .checked_add(HDF5_RANGE_NETWORK_SLICE_BYTES - 1)
                .and_then(|value| value.checked_div(HDF5_RANGE_NETWORK_SLICE_BYTES))
                .ok_or(Hdf5RangeTransferError::RangeBytesExceeded)?;
            request_count = request_count
                .checked_add(slices)
                .ok_or(Hdf5RangeTransferError::RangeCountExceeded)?;
            response_body_bytes = response_body_bytes
                .checked_add(bytes)
                .and_then(|value| value.checked_add(slices))
                .ok_or(Hdf5RangeTransferError::RangeBytesExceeded)?;
        }
        validate_nisar_live_acceptance_budget(request_count, response_body_bytes)?;
        let state = self
            .state
            .lock()
            .map_err(|_| Hdf5RangeTransferError::RangeBytesExceeded)?;
        if state
            .reserved_requests
            .checked_add(request_count)
            .is_none_or(|value| value > NISAR_LIVE_MAX_REQUESTS)
            || state
                .reserved_response_body_bytes
                .checked_add(response_body_bytes)
                .is_none_or(|value| value > self.max_response_body_bytes)
        {
            return Err(Hdf5RangeTransferError::RangeBytesExceeded);
        }
        Ok(())
    }
}

#[cfg(feature = "nisar-subset-production")]
#[doc(hidden)]
pub fn validate_nisar_live_acceptance_budget(
    request_count: u64,
    response_body_bytes: u64,
) -> Result<(), Hdf5RangeTransferError> {
    if request_count > NISAR_LIVE_MAX_REQUESTS {
        return Err(Hdf5RangeTransferError::RangeCountExceeded);
    }
    if response_body_bytes > NISAR_LIVE_MAX_RESPONSE_BODY_BYTES {
        return Err(Hdf5RangeTransferError::RangeBytesExceeded);
    }
    Ok(())
}

/// Build one live Request Manifest from the bounded HDF5 metadata prefix.
///
/// The application service freezes only the source identity and the user's
/// selection.  This function runs after the strong-ETag metadata prefix has
/// been received, so the logical windows are derived from the actual product
/// geolocation grid instead of a hard-coded acceptance fixture.
#[cfg(feature = "nisar-subset-production")]
#[doc(hidden)]
pub fn build_live_nisar_subset_request(
    snapshot_item: &SnapshotItem,
    aoi_geojson: &Map<String, Value>,
    frequency: NisarSubsetFrequency,
    polarization: &str,
    swath: &str,
    sparse_path: &Path,
) -> Result<NisarSubsetRequestManifestV1, Hdf5RangeTransferError> {
    // The live path must execute the same canonical hardened geometry
    // (sparse_grid_polygon_cells_v1) as the provider planner. This function
    // reads the real sparse geolocation grid from the materialized metadata
    // prefix, builds real geolocation evidence, and delegates request planning
    // to the provider `plan_nisar_subset` so there is exactly one geometry
    // implementation and one manifest builder.
    if swath != "LSAR_RSLC" || !supported_dataset_polarizations(&frequency).contains(&polarization)
    {
        return Err(Hdf5RangeTransferError::RangeInvalid);
    }
    let file =
        crate::hdf5_file::open(sparse_path).map_err(|_| Hdf5RangeTransferError::ResponseInvalid)?;
    let science_path = format!(
        "/science/LSAR/RSLC/swaths/frequency{}/{}",
        frequency_label(&frequency),
        polarization
    );
    let science = file
        .dataset(&science_path)
        .map_err(|_| Hdf5RangeTransferError::WindowIdentityMismatch)?;
    let shape = science.shape();
    let chunk = science
        .dcpl()
        .map_err(|_| Hdf5RangeTransferError::RangeInvalid)?
        .get_chunk()
        .map_err(|_| Hdf5RangeTransferError::RangeInvalid)?
        .ok_or(Hdf5RangeTransferError::RangeInvalid)?;
    if shape.len() != 2 || chunk.len() != 2 || shape.contains(&0) || chunk.contains(&0) {
        return Err(Hdf5RangeTransferError::RangeInvalid);
    }
    let source_shape = [shape[0], shape[1]];
    let chunk_shape = [chunk[0], chunk[1]];

    // Real geolocation grid: height layers from heightAboveEllipsoid, and the
    // full coordinateX/coordinateY node axes. The HDF5 grid is
    // [height, line, sample]; node axes are the line/sample indices (sparse
    // node rows/columns are the full radar line/sample grid at the grid
    // spacing). We use the complete coordinate axes as the sparse grid nodes.
    let group = file
        .group("/science/LSAR/RSLC/metadata/geolocationGrid")
        .map_err(|_| Hdf5RangeTransferError::ResponseInvalid)?;
    let height_values: Vec<f64> = group
        .dataset("heightAboveEllipsoid")
        .map_err(|_| Hdf5RangeTransferError::ResponseInvalid)?
        .read_raw()
        .map_err(|_| Hdf5RangeTransferError::ResponseInvalid)?;
    let x_values: Vec<f64> = group
        .dataset("coordinateX")
        .map_err(|_| Hdf5RangeTransferError::ResponseInvalid)?
        .read_raw()
        .map_err(|_| Hdf5RangeTransferError::ResponseInvalid)?;
    let y_values: Vec<f64> = group
        .dataset("coordinateY")
        .map_err(|_| Hdf5RangeTransferError::ResponseInvalid)?
        .read_raw()
        .map_err(|_| Hdf5RangeTransferError::ResponseInvalid)?;
    let x_shape = group
        .dataset("coordinateX")
        .map_err(|_| Hdf5RangeTransferError::ResponseInvalid)?
        .shape();
    if x_shape.len() != 3 || height_values.is_empty() || x_values.len() != y_values.len() {
        return Err(Hdf5RangeTransferError::ResponseInvalid);
    }
    let (height_count, line_count, sample_count) = (x_shape[0], x_shape[1], x_shape[2]);
    if height_count != height_values.len()
        || height_count != 20
        || line_count < 2
        || sample_count < 2
    {
        return Err(Hdf5RangeTransferError::ResponseInvalid);
    }
    let stride = line_count * sample_count;
    let mut longitudes = Vec::with_capacity(height_count);
    let mut latitudes = Vec::with_capacity(height_count);
    for layer in 0..height_count {
        let base = layer * stride;
        longitudes.push(x_values[base..base + stride].to_vec());
        latitudes.push(y_values[base..base + stride].to_vec());
    }
    let min_hae = height_values.iter().copied().fold(f64::INFINITY, f64::min);
    let max_hae = height_values
        .iter()
        .copied()
        .fold(f64::NEG_INFINITY, f64::max);
    let grid_zero_doppler_time: Vec<f64> = group
        .dataset("zeroDopplerTime")
        .map_err(|_| Hdf5RangeTransferError::ResponseInvalid)?
        .read_raw()
        .map_err(|_| Hdf5RangeTransferError::ResponseInvalid)?;
    let grid_slant_range: Vec<f64> = group
        .dataset("slantRange")
        .map_err(|_| Hdf5RangeTransferError::ResponseInvalid)?
        .read_raw()
        .map_err(|_| Hdf5RangeTransferError::ResponseInvalid)?;
    let swath_zero_doppler_time: Vec<f64> = file
        .dataset("/science/LSAR/RSLC/swaths/zeroDopplerTime")
        .map_err(|_| Hdf5RangeTransferError::ResponseInvalid)?
        .read_raw()
        .map_err(|_| Hdf5RangeTransferError::ResponseInvalid)?;
    let swath_slant_range_path = format!(
        "/science/LSAR/RSLC/swaths/frequency{}/slantRange",
        frequency_label(&frequency),
    );
    let swath_slant_range: Vec<f64> = file
        .dataset(&swath_slant_range_path)
        .map_err(|_| Hdf5RangeTransferError::ResponseInvalid)?
        .read_raw()
        .map_err(|_| Hdf5RangeTransferError::ResponseInvalid)?;

    let evidence = insar_provider::asf::nisar_subset::build_live_geolocation_evidence(
        aoi_geojson,
        frequency.clone(),
        polarization,
        swath,
        &grid_zero_doppler_time,
        &grid_slant_range,
        &swath_zero_doppler_time,
        &swath_slant_range,
        height_values,
        longitudes,
        latitudes,
        source_shape,
        chunk_shape,
        min_hae,
        max_hae,
    )
    .map_err(|_| Hdf5RangeTransferError::RangeInvalid)?;

    let request = plan_nisar_subset(
        snapshot_item,
        aoi_geojson,
        frequency,
        polarization,
        swath,
        &evidence,
    )
    .map_err(|_| Hdf5RangeTransferError::WindowIdentityMismatch)?;

    let mut request = request;
    request.expected_output.estimated_remote_bytes = 1;
    let estimated_remote_bytes = estimate_live_nisar_remote_bytes(&request, sparse_path)?;
    request.expected_output.estimated_remote_bytes = estimated_remote_bytes;
    request.expected_output.estimated_output_bytes = estimated_remote_bytes
        .min(request.source.expected_size_bytes)
        .max(1);
    if request.validate().is_err() {
        return Err(Hdf5RangeTransferError::RangeInvalid);
    }
    if validate_request_against_snapshot_item(&request, snapshot_item).is_err() {
        return Err(Hdf5RangeTransferError::WindowIdentityMismatch);
    }
    Ok(request)
}

#[cfg(feature = "nisar-subset-production")]
fn frequency_label(frequency: &NisarSubsetFrequency) -> &'static str {
    match frequency {
        NisarSubsetFrequency::A => "A",
        NisarSubsetFrequency::B => "B",
    }
}

#[cfg(feature = "nisar-subset-production")]
fn estimate_live_nisar_remote_bytes(
    request: &NisarSubsetRequestManifestV1,
    sparse_path: &Path,
) -> Result<u64, Hdf5RangeTransferError> {
    request
        .validate()
        .map_err(|_| Hdf5RangeTransferError::RangeInvalid)?;
    let file =
        crate::hdf5_file::open(sparse_path).map_err(|_| Hdf5RangeTransferError::ResponseInvalid)?;
    let mut physical = Vec::<(u64, u64)>::new();
    let mut datasets = BTreeSet::<(u16, String)>::new();
    for window in &request.mapping.windows {
        let mut paths = vec![window.dataset_path.clone()];
        if window.validity_mask_required {
            paths.push(format!(
                "/science/LSAR/RSLC/swaths/frequency{}/inputDataExceptionMask",
                frequency_label(&window.frequency)
            ));
        }
        for path in paths {
            if !datasets.insert((window.component_id, path.clone())) {
                continue;
            }
            let dataset = file
                .dataset(&path)
                .map_err(|_| Hdf5RangeTransferError::WindowIdentityMismatch)?;
            let shape = dataset.shape();
            let chunk = dataset
                .dcpl()
                .map_err(|_| Hdf5RangeTransferError::RangeInvalid)?
                .get_chunk()
                .map_err(|_| Hdf5RangeTransferError::RangeInvalid)?
                .ok_or(Hdf5RangeTransferError::RangeInvalid)?;
            if shape.len() != 2 || chunk.len() != 2 {
                return Err(Hdf5RangeTransferError::RangeInvalid);
            }
            let dataset_shape = [shape[0] as u64, shape[1] as u64];
            let chunk_shape = [chunk[0] as u64, chunk[1] as u64];
            if dataset_shape.contains(&0)
                || chunk_shape.contains(&0)
                || dataset_shape[0] < window.line_end_exclusive
                || dataset_shape[1] < window.sample_end_exclusive
            {
                return Err(Hdf5RangeTransferError::WindowIdentityMismatch);
            }
            let datatype = if path == window.dataset_path {
                Hdf5ChunkDatatypeV1::ComplexF32LittleEndianRi
            } else {
                Hdf5ChunkDatatypeV1::UnsignedU8
            };
            let _ = datatype;
            // Downloader enhancement D1d: one pass over the chunk index.
            crate::hdf5_chunk_visit::visit_stored_chunks(&dataset, |chunk| {
                if chunk.offset.len() != 2 {
                    return Err(Hdf5RangeTransferError::RangeInvalid);
                }
                let line_start = chunk.offset[0];
                let sample_start = chunk.offset[1];
                let line_end = line_start
                    .checked_add(chunk_shape[0])
                    .ok_or(Hdf5RangeTransferError::RangeBytesExceeded)?;
                let sample_end = sample_start
                    .checked_add(chunk_shape[1])
                    .ok_or(Hdf5RangeTransferError::RangeBytesExceeded)?;
                if line_start >= window.line_end_exclusive
                    || line_end <= window.line_start
                    || sample_start >= window.sample_end_exclusive
                    || sample_end <= window.sample_start
                {
                    return Ok(());
                }
                let end = chunk
                    .address
                    .checked_add(chunk.size)
                    .ok_or(Hdf5RangeTransferError::RangeBytesExceeded)?;
                if chunk.address == u64::MAX || chunk.filter_mask != 0 {
                    return Err(Hdf5RangeTransferError::RangeInvalid);
                }
                // Real product stores chunk payloads sparsely (physical
                // size < logical capacity); download exactly the stored size.
                // Science chunks physically inside the metadata prefix are
                // already materialized by the prefix download. Their bytes are
                // counted inside the prefix total; only the portion at or
                // after the prefix needs a dedicated science Range.
                if end <= NISAR_LIVE_METADATA_PREFIX_BYTES {
                    return Ok(());
                }
                physical.push((chunk.address.max(NISAR_LIVE_METADATA_PREFIX_BYTES), end));
                Ok(())
            })
            .map_err(|error| match error {
                crate::hdf5_chunk_visit::ChunkVisitError::Ended(error) => error,
                _ => Hdf5RangeTransferError::RangeInvalid,
            })?;
        }
    }
    if physical.is_empty() {
        return Err(Hdf5RangeTransferError::RangeInvalid);
    }
    physical.sort_unstable();
    let mut total = NISAR_LIVE_METADATA_PREFIX_BYTES;
    let mut current = physical[0];
    for next in physical.into_iter().skip(1) {
        if next.0 < current.1 {
            return Err(Hdf5RangeTransferError::RangeOverlap);
        }
        if next.0 == current.1 {
            current.1 = next.1;
        } else {
            total = total
                .checked_add(current.1 - current.0)
                .ok_or(Hdf5RangeTransferError::RangeBytesExceeded)?;
            current = next;
        }
    }
    total = total
        .checked_add(current.1 - current.0)
        .ok_or(Hdf5RangeTransferError::RangeBytesExceeded)?;
    if total == 0 || total > HDF5_RANGE_MAX_REMOTE_BYTES {
        return Err(Hdf5RangeTransferError::RangeBytesExceeded);
    }
    Ok(total)
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(deny_unknown_fields)]
/// Crate-internal offline layout evidence. `Hdf5RangePlanV1::from_offline_evidence`
/// revalidates every field; this DTO is not an execution capability.
pub(crate) struct Hdf5ChunkPhysicalRangeV1 {
    pub(crate) dataset_path: String,
    pub(crate) component_id: u16,
    pub(crate) logical_chunk: [u64; 2],
    pub(crate) dataset_shape: [u64; 2],
    pub(crate) chunk_shape: [u64; 2],
    pub(crate) datatype: Hdf5ChunkDatatypeV1,
    pub(crate) datatype_sha256: String,
    pub(crate) fill_value_definition: Hdf5FillValueDefinitionV1,
    pub(crate) fill_value_hex: Option<String>,
    pub(crate) filter_pipeline: Hdf5ChunkFilterPipelineV1,
    pub(crate) allocation: Hdf5ChunkAllocationV1,
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(rename_all = "snake_case")]
pub(crate) enum Hdf5ChunkDatatypeV1 {
    ComplexF32LittleEndianRi,
    UnsignedU8,
}

impl Hdf5ChunkDatatypeV1 {
    pub(crate) const fn element_size(self) -> usize {
        match self {
            Self::ComplexF32LittleEndianRi => 8,
            Self::UnsignedU8 => 1,
        }
    }
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(rename_all = "snake_case")]
pub(crate) enum Hdf5FillValueDefinitionV1 {
    Undefined,
    Default,
    UserDefined,
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(rename_all = "snake_case")]
pub(crate) enum Hdf5ChunkFilterPipelineV1 {
    None,
    /// Reviewed built-in pipeline: shuffle then deflate.
    /// For the admitted P05023 / spec-1.5.0 object the accepted level is 4.
    ShuffleDeflate {
        level: u8,
    },
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(tag = "allocation", rename_all = "snake_case", deny_unknown_fields)]
pub(crate) enum Hdf5ChunkAllocationV1 {
    Allocated {
        filter_mask: u32,
        stored_size: u64,
        start: u64,
        end_exclusive: u64,
    },
    Fill {
        element_fill_hex: String,
    },
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
/// Offline evidence envelope awaiting deterministic Request/window validation.
/// Only the sealed, non-deserializable `Hdf5RangePlanV1` can execute.
pub(crate) struct ApprovedHdf5ChunkSetV1 {
    pub(crate) request_manifest_sha256: String,
    pub(crate) source_length: u64,
    pub(crate) layout_evidence_digest: String,
    pub(crate) metadata_evidence_sha256: String,
    pub(crate) chunks: Vec<Hdf5ChunkPhysicalRangeV1>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct VerifiedHdf5ChunkBlockV1 {
    pub(crate) dataset_path: String,
    pub(crate) component_id: u16,
    pub(crate) logical_chunk: [u64; 2],
    pub(crate) dataset_shape: [u64; 2],
    pub(crate) chunk_shape: [u64; 2],
    pub(crate) datatype: Hdf5ChunkDatatypeV1,
    pub(crate) datatype_sha256: String,
    pub(crate) fill_value_definition: Hdf5FillValueDefinitionV1,
    pub(crate) fill_value_hex: Option<String>,
    pub(crate) filter_pipeline: Hdf5ChunkFilterPipelineV1,
    pub(crate) part_path: PathBuf,
    pub(crate) allocation: VerifiedHdf5ChunkAllocationV1,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum VerifiedHdf5ChunkAllocationV1 {
    Allocated { bytes: Arc<[u8]> },
    Fill { element_fill_hex: String },
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Hdf5PhysicalRangeV1 {
    pub start: u64,
    pub end_exclusive: u64,
    member_chunk_indices: Vec<u32>,
}

#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
pub struct Hdf5RangePlanV1 {
    plan_contract: String,
    source_length: u64,
    #[serde(default)]
    metadata_prefix_bytes: u64,
    chunks: Vec<Hdf5ChunkPhysicalRangeV1>,
    ranges: Vec<Hdf5PhysicalRangeV1>,
    expected_remote_bytes: u64,
    request_manifest_sha256: String,
    layout_evidence_digest: String,
    metadata_evidence_sha256: String,
    plan_sha256: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct StoredHdf5RangePlanV1 {
    plan_contract: String,
    source_length: u64,
    #[serde(default)]
    metadata_prefix_bytes: u64,
    chunks: Vec<Hdf5ChunkPhysicalRangeV1>,
    ranges: Vec<Hdf5PhysicalRangeV1>,
    expected_remote_bytes: u64,
    request_manifest_sha256: String,
    layout_evidence_digest: String,
    metadata_evidence_sha256: String,
    plan_sha256: String,
}

impl StoredHdf5RangePlanV1 {
    fn into_plan(self) -> Hdf5RangePlanV1 {
        Hdf5RangePlanV1 {
            plan_contract: self.plan_contract,
            source_length: self.source_length,
            metadata_prefix_bytes: self.metadata_prefix_bytes,
            chunks: self.chunks,
            ranges: self.ranges,
            expected_remote_bytes: self.expected_remote_bytes,
            request_manifest_sha256: self.request_manifest_sha256,
            layout_evidence_digest: self.layout_evidence_digest,
            metadata_evidence_sha256: self.metadata_evidence_sha256,
            plan_sha256: self.plan_sha256,
        }
    }
}

#[derive(Clone, Copy, Debug, Error, PartialEq, Eq)]
pub enum Hdf5RangeTransferError {
    #[error("NISAR subset physical range does not map to a requested logical window")]
    WindowIdentityMismatch,
    #[error("NISAR subset physical ranges overlap")]
    RangeOverlap,
    #[error("NISAR subset logical chunk identity is duplicated")]
    ChunkIdentityDuplicate,
    #[error("NISAR subset physical range count exceeds the offline bound")]
    RangeCountExceeded,
    #[error("NISAR subset remote-byte plan exceeds the offline bound")]
    RangeBytesExceeded,
    #[error("NISAR subset physical range shape is invalid")]
    RangeInvalid,
    #[error("NISAR subset physical byte total differs from the immutable request")]
    RemoteByteTotalMismatch,
    #[error("NISAR subset Range workspace is invalid")]
    WorkspaceInvalid,
    #[error("NISAR subset Range workspace is already owned")]
    WorkspaceBusy,
    #[error("NISAR subset Range requires a frozen strong ETag")]
    StrongValidatorRequired,
    #[error("NISAR subset Range response used a weak ETag")]
    WeakEtagRejected,
    #[error("NISAR subset server ignored the exact Range request")]
    RangeUnsupported,
    #[error("NISAR subset Range is not satisfiable")]
    RangeNotSatisfiable,
    #[error("NISAR subset Range response is malformed")]
    ResponseInvalid,
    #[error("NISAR subset Range response has malformed Content-Range")]
    MalformedContentRange,
    #[error("NISAR subset source requires Earthdata credentials or permission")]
    CredentialsRequired,
    #[error("NISAR subset source validator changed")]
    SourceChanged,
    #[error("NISAR subset Range transfer was cancelled")]
    Cancelled,
    #[error("NISAR subset Range response was partial")]
    PartialResponse,
    #[error("NISAR subset Range transport failed")]
    Transport,
    #[error("NISAR subset Range checkpoint could not be persisted")]
    Persistence,
    #[error("NISAR subset Range recovery evidence is invalid")]
    RecoveryInvalid,
}

impl Hdf5RangeTransferError {
    pub const fn code(self) -> &'static str {
        match self {
            Self::RangeUnsupported | Self::StrongValidatorRequired | Self::WeakEtagRejected => {
                "NISAR_SUBSET_NOT_SUPPORTED"
            }
            Self::SourceChanged => "NISAR_SUBSET_SOURCE_CHANGED",
            Self::Cancelled | Self::PartialResponse | Self::Transport => "NISAR_SUBSET_INCOMPLETE",
            Self::WorkspaceInvalid
            | Self::WorkspaceBusy
            | Self::Persistence
            | Self::RecoveryInvalid
            | Self::WindowIdentityMismatch
            | Self::RangeOverlap
            | Self::ChunkIdentityDuplicate
            | Self::RangeCountExceeded
            | Self::RangeBytesExceeded
            | Self::RangeInvalid
            | Self::RemoteByteTotalMismatch
            | Self::RangeNotSatisfiable
            | Self::ResponseInvalid
            | Self::MalformedContentRange => "NISAR_SUBSET_RANGE_INVALID",
            Self::CredentialsRequired => "NISAR_SUBSET_CREDENTIAL_OR_PERMISSION",
        }
    }
}

impl Hdf5RangePlanV1 {
    pub(crate) fn from_offline_evidence(
        request: &NisarSubsetRequestManifestV1,
        evidence: ApprovedHdf5ChunkSetV1,
    ) -> Result<Self, Hdf5RangeTransferError> {
        Self::from_evidence_with_metadata_prefix(request, evidence, 0)
    }

    #[cfg(any(test, feature = "offline-nisar-subset-test-support"))]
    pub(crate) fn from_evidence_with_metadata_prefix_for_test(
        request: &NisarSubsetRequestManifestV1,
        evidence: ApprovedHdf5ChunkSetV1,
        metadata_prefix_bytes: u64,
    ) -> Result<Self, Hdf5RangeTransferError> {
        Self::from_evidence_with_metadata_prefix(request, evidence, metadata_prefix_bytes)
    }

    fn push_or_merge_range(
        normalized: &mut Vec<Hdf5PhysicalRangeV1>,
        last_physical: &mut Option<(u64, u64, u32)>,
        index: u32,
        start: u64,
        end_exclusive: u64,
        chunks: &[Hdf5ChunkPhysicalRangeV1],
    ) -> Result<(), Hdf5RangeTransferError> {
        if let Some((previous_start, previous_end, previous_index)) = *last_physical {
            if start == previous_start && end_exclusive == previous_end {
                let previous_chunk = usize::try_from(previous_index)
                    .ok()
                    .and_then(|index| chunks.get(index));
                let current_chunk = usize::try_from(index)
                    .ok()
                    .and_then(|index| chunks.get(index));
                if previous_chunk
                    .zip(current_chunk)
                    .is_some_and(|(left, right)| same_component_agnostic_chunk(left, right))
                {
                    normalized
                        .last_mut()
                        .ok_or(Hdf5RangeTransferError::RangeInvalid)?
                        .member_chunk_indices
                        .push(index);
                    return Ok(());
                }
                return Err(Hdf5RangeTransferError::RangeOverlap);
            }
            if start < previous_end {
                return Err(Hdf5RangeTransferError::RangeOverlap);
            }
        }
        let Some(previous) = normalized.last_mut() else {
            normalized.push(Hdf5PhysicalRangeV1 {
                start,
                end_exclusive,
                member_chunk_indices: vec![index],
            });
            *last_physical = Some((start, end_exclusive, index));
            return Ok(());
        };
        if start == previous.end_exclusive {
            previous.end_exclusive = end_exclusive;
            previous.member_chunk_indices.push(index);
        } else {
            normalized.push(Hdf5PhysicalRangeV1 {
                start,
                end_exclusive,
                member_chunk_indices: vec![index],
            });
        }
        *last_physical = Some((start, end_exclusive, index));
        Ok(())
    }

    fn from_evidence_with_metadata_prefix(
        request: &NisarSubsetRequestManifestV1,
        mut evidence: ApprovedHdf5ChunkSetV1,
        metadata_prefix_bytes: u64,
    ) -> Result<Self, Hdf5RangeTransferError> {
        request
            .validate()
            .map_err(|_| Hdf5RangeTransferError::RangeInvalid)?;
        let request_manifest_sha256 = request
            .sha256()
            .map_err(|_| Hdf5RangeTransferError::RangeInvalid)?;
        if evidence.request_manifest_sha256 != request_manifest_sha256
            || evidence.source_length != request.expected_output.full_product_bytes
            || metadata_prefix_bytes > evidence.source_length
            || !is_sha256(&evidence.layout_evidence_digest)
            || !is_sha256(&evidence.metadata_evidence_sha256)
        {
            return Err(Hdf5RangeTransferError::WindowIdentityMismatch);
        }
        let mut chunks = std::mem::take(&mut evidence.chunks);
        if chunks.is_empty() || chunks.len() > HDF5_RANGE_MAX_COUNT {
            return Err(if chunks.len() > HDF5_RANGE_MAX_COUNT {
                Hdf5RangeTransferError::RangeCountExceeded
            } else {
                Hdf5RangeTransferError::RangeInvalid
            });
        }
        for chunk in &chunks {
            if !chunk_maps_to_request(request, chunk) {
                return Err(Hdf5RangeTransferError::WindowIdentityMismatch);
            }
            validate_chunk_dataset_identity(request, chunk)?;
            match &chunk.allocation {
                Hdf5ChunkAllocationV1::Allocated {
                    filter_mask,
                    stored_size,
                    start,
                    end_exclusive,
                } => {
                    if start >= end_exclusive
                        || *end_exclusive > request.expected_output.full_product_bytes
                        || *stored_size != end_exclusive - start
                        || *filter_mask != 0
                    {
                        return Err(Hdf5RangeTransferError::RangeInvalid);
                    }
                }
                Hdf5ChunkAllocationV1::Fill { element_fill_hex } => {
                    if Some(element_fill_hex.as_str()) != chunk.fill_value_hex.as_deref()
                        || element_fill_hex.is_empty()
                        || element_fill_hex.len() > 64
                        || element_fill_hex.len() % 2 != 0
                        || !element_fill_hex
                            .bytes()
                            .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
                    {
                        return Err(Hdf5RangeTransferError::RangeInvalid);
                    }
                }
            }
        }
        let mut logical_identities = BTreeSet::new();
        for chunk in &chunks {
            if !logical_identities.insert((
                chunk.component_id,
                chunk.dataset_path.as_str(),
                chunk.logical_chunk,
            )) {
                return Err(Hdf5RangeTransferError::ChunkIdentityDuplicate);
            }
        }
        validate_science_chunk_coverage(request, &chunks)?;
        chunks.sort();

        let mut allocated = chunks
            .iter()
            .enumerate()
            .filter_map(|(index, chunk)| match &chunk.allocation {
                Hdf5ChunkAllocationV1::Allocated {
                    start,
                    end_exclusive,
                    ..
                } => Some((index, *start, *end_exclusive)),
                Hdf5ChunkAllocationV1::Fill { .. } => None,
            })
            .collect::<Vec<_>>();
        allocated.sort_by_key(|(_, start, end_exclusive)| (*start, *end_exclusive));
        // Science chunks physically inside the metadata prefix are already
        // materialized by the prefix download. They must not be downloaded
        // again: their member indices join the prefix range (range[0]), whose
        // bytes already exist in the part file. Only chunks at or after the
        // prefix become dedicated science ranges.
        let mut normalized: Vec<Hdf5PhysicalRangeV1> = Vec::with_capacity(allocated.len());
        let mut last_physical: Option<(u64, u64, u32)> = None;
        for (index, start, end_exclusive) in allocated {
            let index =
                u32::try_from(index).map_err(|_| Hdf5RangeTransferError::RangeCountExceeded)?;
            if end_exclusive <= metadata_prefix_bytes {
                // Fully prefix-resident chunks need no dedicated Range; their
                // bytes are owned by the prefix range and read from the part
                // during block materialization.
                continue;
            }
            if start < metadata_prefix_bytes {
                // Chunk straddles the prefix boundary: the prefix part is
                // already materialized, the remainder must be downloaded.
                // Keep the member in the science range and rely on the
                // overlap extraction to read only the downloaded part from
                // the completed science range (the prefix portion is already
                // present in the part file at its true offset).
                let cropped_start = metadata_prefix_bytes;
                if cropped_start >= end_exclusive {
                    continue;
                }
                let start = cropped_start;
                Self::push_or_merge_range(
                    &mut normalized,
                    &mut last_physical,
                    index,
                    start,
                    end_exclusive,
                    &chunks,
                )?;
                continue;
            }
            Self::push_or_merge_range(
                &mut normalized,
                &mut last_physical,
                index,
                start,
                end_exclusive,
                &chunks,
            )?;
        }
        if normalized.len() > HDF5_RANGE_MAX_COUNT {
            return Err(Hdf5RangeTransferError::RangeCountExceeded);
        }
        if metadata_prefix_bytes > 0 {
            normalized.insert(
                0,
                Hdf5PhysicalRangeV1 {
                    start: 0,
                    end_exclusive: metadata_prefix_bytes,
                    // Prefix-owned chunks are kept in `plan.chunks`; the
                    // prefix range itself owns no member list so the metadata
                    // seed contract (empty prefix members) is preserved.
                    member_chunk_indices: Vec::new(),
                },
            );
        }
        if normalized.len() > HDF5_RANGE_MAX_COUNT {
            return Err(Hdf5RangeTransferError::RangeCountExceeded);
        }
        let expected_remote_bytes = normalized.iter().try_fold(0_u64, |total, range| {
            total.checked_add(range.end_exclusive - range.start)
        });
        let Some(expected_remote_bytes) = expected_remote_bytes else {
            return Err(Hdf5RangeTransferError::RangeBytesExceeded);
        };
        if expected_remote_bytes > HDF5_RANGE_MAX_REMOTE_BYTES {
            return Err(Hdf5RangeTransferError::RangeBytesExceeded);
        }
        if expected_remote_bytes != request.expected_output.estimated_remote_bytes {
            return Err(Hdf5RangeTransferError::RemoteByteTotalMismatch);
        }
        let mut plan = Self {
            plan_contract: "NisarOfflineHdf5RangePlanV1".to_owned(),
            source_length: evidence.source_length,
            metadata_prefix_bytes,
            chunks,
            ranges: normalized,
            expected_remote_bytes,
            request_manifest_sha256,
            layout_evidence_digest: evidence.layout_evidence_digest,
            metadata_evidence_sha256: evidence.metadata_evidence_sha256,
            plan_sha256: String::new(),
        };
        plan.plan_sha256 = compute_plan_sha256(&plan)?;
        Ok(plan)
    }

    pub fn ranges(&self) -> &[Hdf5PhysicalRangeV1] {
        &self.ranges
    }

    pub(crate) fn chunks(&self) -> &[Hdf5ChunkPhysicalRangeV1] {
        &self.chunks
    }

    pub const fn expected_remote_bytes(&self) -> u64 {
        self.expected_remote_bytes
    }

    pub const fn metadata_prefix_bytes(&self) -> u64 {
        self.metadata_prefix_bytes
    }

    pub const fn source_length(&self) -> u64 {
        self.source_length
    }

    pub fn request_manifest_sha256(&self) -> &str {
        &self.request_manifest_sha256
    }

    #[allow(dead_code)]
    pub fn layout_evidence_digest(&self) -> &str {
        &self.layout_evidence_digest
    }

    pub fn plan_sha256(&self) -> &str {
        &self.plan_sha256
    }

    pub fn metadata_evidence_sha256(&self) -> &str {
        &self.metadata_evidence_sha256
    }

    /// Re-check the complete physical-plan contract immediately before live
    /// admission.  Construction already seals this identity, but the live
    /// gate must make each physical invariant explicit instead of treating a
    /// semantic SHA-256 as a substitute for validation.
    pub(crate) fn validate_nisar_live_physical_admission(
        &self,
        request: &NisarSubsetRequestManifestV1,
    ) -> Result<(), Hdf5RangeTransferError> {
        request
            .validate()
            .map_err(|_| Hdf5RangeTransferError::RangeInvalid)?;
        let request_manifest_sha256 = request
            .sha256()
            .map_err(|_| Hdf5RangeTransferError::RangeInvalid)?;
        if self.plan_contract != "NisarOfflineHdf5RangePlanV1"
            || self.request_manifest_sha256 != request_manifest_sha256
            || self.source_length != request.source.expected_size_bytes
            || self.source_length != request.expected_output.full_product_bytes
            || self.metadata_prefix_bytes > self.source_length
            || !is_sha256(&self.layout_evidence_digest)
            || !is_sha256(&self.metadata_evidence_sha256)
            || !is_sha256(&self.plan_sha256)
            || compute_plan_sha256(self)? != self.plan_sha256
        {
            return Err(Hdf5RangeTransferError::WindowIdentityMismatch);
        }
        if self.chunks.is_empty() || self.chunks.len() > HDF5_RANGE_MAX_COUNT {
            return Err(Hdf5RangeTransferError::RangeInvalid);
        }

        let mut logical_identities = BTreeSet::new();
        for chunk in &self.chunks {
            if !chunk_maps_to_request(request, chunk) {
                return Err(Hdf5RangeTransferError::WindowIdentityMismatch);
            }
            validate_chunk_dataset_identity(request, chunk)?;
            if !logical_identities.insert((
                chunk.component_id,
                chunk.dataset_path.as_str(),
                chunk.logical_chunk,
            )) {
                return Err(Hdf5RangeTransferError::ChunkIdentityDuplicate);
            }
            match &chunk.allocation {
                Hdf5ChunkAllocationV1::Allocated {
                    filter_mask,
                    stored_size,
                    start,
                    end_exclusive,
                } => {
                    if *start >= *end_exclusive
                        || *end_exclusive > self.source_length
                        || *stored_size != end_exclusive - start
                        || *filter_mask != 0
                    {
                        return Err(Hdf5RangeTransferError::RangeInvalid);
                    }
                }
                Hdf5ChunkAllocationV1::Fill { element_fill_hex } => {
                    if Some(element_fill_hex.as_str()) != chunk.fill_value_hex.as_deref()
                        || element_fill_hex.is_empty()
                        || element_fill_hex.len() > 64
                        || element_fill_hex.len() % 2 != 0
                        || !element_fill_hex
                            .bytes()
                            .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
                    {
                        return Err(Hdf5RangeTransferError::RangeInvalid);
                    }
                }
            }
        }
        validate_science_chunk_coverage(request, &self.chunks)?;

        if self.ranges.is_empty() || self.ranges.len() > HDF5_RANGE_MAX_COUNT {
            return Err(Hdf5RangeTransferError::RangeInvalid);
        }
        let mut previous_end = 0_u64;
        let mut total = 0_u64;
        for (index, range) in self.ranges.iter().enumerate() {
            if range.start >= range.end_exclusive
                || range.end_exclusive > self.source_length
                || (index > 0 && range.start < previous_end)
            {
                return Err(Hdf5RangeTransferError::RangeInvalid);
            }
            if self.metadata_prefix_bytes > 0
                && index == 0
                && (range.start != 0 || range.end_exclusive != self.metadata_prefix_bytes)
            {
                return Err(Hdf5RangeTransferError::RangeInvalid);
            }
            previous_end = range.end_exclusive;
            total = total
                .checked_add(range.end_exclusive - range.start)
                .ok_or(Hdf5RangeTransferError::RangeBytesExceeded)?;
        }
        if total == 0
            || total != self.expected_remote_bytes
            || total != request.expected_output.estimated_remote_bytes
            || total > HDF5_RANGE_MAX_REMOTE_BYTES
        {
            return Err(Hdf5RangeTransferError::RemoteByteTotalMismatch);
        }
        Ok(())
    }

    #[cfg(test)]
    pub(crate) fn with_test_datatype_mutation(&self) -> Self {
        let mut mutated = self.clone();
        let chunk = mutated
            .chunks
            .first_mut()
            .expect("semantic test plan must contain a chunk");
        chunk.datatype = Hdf5ChunkDatatypeV1::UnsignedU8;
        chunk.datatype_sha256 =
            supported_datatype_sha256(Hdf5ChunkDatatypeV1::UnsignedU8).expect("u8 datatype digest");
        mutated.plan_sha256 = compute_plan_sha256(&mutated).expect("mutated plan digest");
        mutated
    }

    #[cfg(test)]
    pub(crate) fn with_test_filter_mutation(&self) -> Self {
        let mut mutated = self.clone();
        let chunk = mutated
            .chunks
            .first_mut()
            .expect("semantic test plan must contain a chunk");
        chunk.filter_pipeline = Hdf5ChunkFilterPipelineV1::ShuffleDeflate { level: 4 };
        mutated.plan_sha256 = compute_plan_sha256(&mutated).expect("mutated plan digest");
        mutated
    }

    #[cfg(test)]
    #[allow(dead_code)]
    pub(crate) fn with_test_unqualified_filter_mutation(&self) -> Self {
        let mut mutated = self.clone();
        let chunk = mutated
            .chunks
            .first_mut()
            .expect("semantic test plan must contain a chunk");
        chunk.filter_pipeline = Hdf5ChunkFilterPipelineV1::ShuffleDeflate { level: 9 };
        mutated.plan_sha256 = compute_plan_sha256(&mutated).expect("mutated plan digest");
        mutated
    }

    #[cfg(test)]
    #[allow(dead_code)]
    pub(crate) fn with_test_malformed_range_mutation(&self) -> Self {
        let mut mutated = self.clone();
        let range = mutated
            .ranges
            .first_mut()
            .expect("range test plan must contain a range");
        range.end_exclusive = range.start;
        mutated.plan_sha256 = compute_plan_sha256(&mutated).expect("mutated plan digest");
        mutated
    }

    #[cfg(test)]
    #[allow(dead_code)]
    pub(crate) fn with_test_source_bound_mutation(&self) -> Self {
        let mut mutated = self.clone();
        mutated.source_length = mutated.source_length.saturating_sub(1);
        mutated.plan_sha256 = compute_plan_sha256(&mutated).expect("mutated plan digest");
        mutated
    }

    #[cfg(test)]
    #[allow(dead_code)]
    pub(crate) fn with_test_incomplete_coverage_mutation(&self) -> Self {
        let mut mutated = self.clone();
        mutated.chunks.pop();
        mutated.plan_sha256 = compute_plan_sha256(&mutated).expect("mutated plan digest");
        mutated
    }
}

/// Half-open verified acquisition interval.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct VerifiedCoverage {
    start: u64,
    end_exclusive: u64,
}

/// Normalize completed, digest-verified acquisition intervals into a sorted,
/// merged (adjacent or overlapping) coverage set.
///
/// Requirements enforced here: half-open intervals `[start,end)`, zero-width
/// intervals rejected, deterministic ordering, source-bound checked, and
/// adjacent/overlapping intervals merged so `interval_is_fully_covered` can
/// test containment against one disjoint coverage list.
fn normalize_verified_ranges(
    ranges: &[(u64, u64)],
    source_length: u64,
) -> Result<Vec<VerifiedCoverage>, Hdf5RangeTransferError> {
    if ranges.is_empty() {
        return Ok(Vec::new());
    }
    for (start, end_exclusive) in ranges {
        if start >= end_exclusive || *end_exclusive > source_length {
            return Err(Hdf5RangeTransferError::RecoveryInvalid);
        }
    }
    let mut sorted = ranges.to_vec();
    sorted.sort_unstable();
    let mut normalized = Vec::<VerifiedCoverage>::with_capacity(sorted.len());
    for (start, end_exclusive) in sorted {
        if let Some(last) = normalized.last_mut() {
            if start <= last.end_exclusive {
                if end_exclusive > last.end_exclusive {
                    last.end_exclusive = end_exclusive;
                }
                continue;
            }
        }
        normalized.push(VerifiedCoverage {
            start,
            end_exclusive,
        });
    }
    Ok(normalized)
}

/// True iff `[start,end)` is fully contained in the union of normalized
/// verified coverage.
fn interval_is_fully_covered(interval: (u64, u64), coverage: &[VerifiedCoverage]) -> bool {
    let (start, end_exclusive) = interval;
    if start >= end_exclusive {
        return false;
    }
    let index = coverage.partition_point(|range| range.end_exclusive <= start);
    coverage
        .get(index)
        .is_some_and(|range| range.start <= start && range.end_exclusive >= end_exclusive)
}

#[cfg(feature = "nisar-subset-production")]
/// Runtime-only facts from the one-byte live validator preflight.  The raw
/// validator is intentionally accessible only by consuming this value and is
/// never serializable or printable through `Debug`.
#[doc(hidden)]
pub struct NisarLivePreflightV1 {
    source_validator: NisarSubsetSourceValidatorV1,
    final_host_class: &'static str,
    final_path_class: &'static str,
    locator_hash: String,
    validator_hash: String,
    request_count: u64,
    response_body_bytes: u64,
}

#[cfg(feature = "nisar-subset-production")]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[doc(hidden)]
pub(crate) struct NisarLiveTransportAdmissionV1 {
    pub(crate) final_host_class: &'static str,
    pub(crate) final_path_class: &'static str,
    pub(crate) range_status_206: bool,
    pub(crate) content_range_exact: bool,
    pub(crate) response_length_exact: bool,
    pub(crate) full_file_fallback_rejected: bool,
    pub(crate) unauthorized_redirect_rejected: bool,
}

#[cfg(feature = "nisar-subset-production")]
impl NisarLivePreflightV1 {
    pub(crate) fn admission_facts(
        &self,
        expected_size: u64,
    ) -> Result<NisarLiveTransportAdmissionV1, Hdf5RangeTransferError> {
        let validator = &self.source_validator;
        if expected_size == 0
            || validator.kind != NisarSubsetValidatorKind::StrongEtag
            || validator.value.starts_with("W/")
            || !validator.value.starts_with('"')
            || !validator.value.ends_with('"')
            || validator.value.len() < 3
            || validator.content_length != expected_size
            || self.final_host_class != "earthdata_asf_v1"
            || self.final_path_class != "approved_nisar_hdf5_object"
        {
            return Err(Hdf5RangeTransferError::ResponseInvalid);
        }
        Ok(NisarLiveTransportAdmissionV1 {
            final_host_class: self.final_host_class,
            final_path_class: self.final_path_class,
            range_status_206: true,
            content_range_exact: true,
            response_length_exact: true,
            full_file_fallback_rejected: true,
            unauthorized_redirect_rejected: true,
        })
    }

    pub fn into_source_validator(self) -> NisarSubsetSourceValidatorV1 {
        self.source_validator
    }

    pub fn final_host_class(&self) -> &'static str {
        self.final_host_class
    }

    pub fn final_path_class(&self) -> &'static str {
        self.final_path_class
    }

    pub fn locator_hash(&self) -> &str {
        &self.locator_hash
    }

    pub fn validator_hash(&self) -> &str {
        &self.validator_hash
    }

    pub const fn request_count(&self) -> u64 {
        self.request_count
    }

    pub const fn response_body_bytes(&self) -> u64 {
        self.response_body_bytes
    }
}

#[cfg(feature = "nisar-subset-production")]
impl std::fmt::Debug for NisarLivePreflightV1 {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("NisarLivePreflightV1")
            .field("final_host_class", &self.final_host_class)
            .field("final_path_class", &self.final_path_class)
            .field("locator_hash", &self.locator_hash)
            .field("validator_hash", &self.validator_hash)
            .field("request_count", &self.request_count)
            .field("response_body_bytes", &self.response_body_bytes)
            .field("source_validator", &"[RUNTIME_ONLY]")
            .finish()
    }
}

#[cfg(feature = "nisar-subset-production")]
#[derive(Clone, Debug, PartialEq, Eq)]
#[doc(hidden)]
pub struct NisarLiveMetadataPrefetchV1 {
    request_count: u64,
    response_body_bytes: u64,
    prefix_sha256: String,
}

#[cfg(feature = "nisar-subset-production")]
impl NisarLiveMetadataPrefetchV1 {
    pub const fn request_count(&self) -> u64 {
        self.request_count
    }

    pub const fn response_body_bytes(&self) -> u64 {
        self.response_body_bytes
    }

    pub fn prefix_sha256(&self) -> &str {
        &self.prefix_sha256
    }
}

#[cfg(feature = "nisar-subset-production")]
#[doc(hidden)]
pub fn preflight_live_nisar_source(
    http: &HttpTransfer,
    asset: &ResolvedAsset,
    expected_size: u64,
    cancelled: &(dyn Fn() -> bool + Sync),
) -> Result<NisarLivePreflightV1, Hdf5RangeTransferError> {
    preflight_live_nisar_source_with_budget(http, asset, expected_size, cancelled, None)
}

#[cfg(feature = "nisar-subset-production")]
#[doc(hidden)]
pub(crate) fn preflight_live_nisar_source_with_budget(
    http: &HttpTransfer,
    asset: &ResolvedAsset,
    expected_size: u64,
    cancelled: &(dyn Fn() -> bool + Sync),
    budget: Option<&NisarLiveNetworkBudgetV1>,
) -> Result<NisarLivePreflightV1, Hdf5RangeTransferError> {
    if expected_size == 0 || asset.method() != crate::ResolvedMethod::Get {
        return Err(Hdf5RangeTransferError::RangeUnsupported);
    }
    if let Some(budget) = budget {
        budget.reserve_response_body_bytes(2)?;
    }
    let outcome = match budget {
        Some(budget) => {
            http.send_resolved_preflight_range_with_nisar_budget(asset, 0, 1, cancelled, budget)
        }
        None => http.send_resolved_preflight_range(asset, 0, 1, cancelled),
    };
    let (mut response, request_count) = match outcome {
        ExactRangeSendOutcome::Response {
            response,
            request_count,
        } => (response, request_count),
        ExactRangeSendOutcome::Cancelled { .. } => return Err(Hdf5RangeTransferError::Cancelled),
        ExactRangeSendOutcome::RetryableExhausted { .. }
        | ExactRangeSendOutcome::ProtocolRejected { .. }
        | ExactRangeSendOutcome::TrustedRedirectRejected { .. } => {
            return Err(Hdf5RangeTransferError::ResponseInvalid)
        }
        ExactRangeSendOutcome::BudgetExceeded { .. } => {
            return Err(Hdf5RangeTransferError::RangeCountExceeded)
        }
    };
    if matches!(
        response.status(),
        StatusCode::UNAUTHORIZED | StatusCode::FORBIDDEN
    ) {
        return Err(Hdf5RangeTransferError::CredentialsRequired);
    }
    let etag = response_strong_etag(&response)?;
    if response.status() != StatusCode::PARTIAL_CONTENT {
        return Err(if response.status() == StatusCode::OK {
            Hdf5RangeTransferError::RangeUnsupported
        } else {
            Hdf5RangeTransferError::ResponseInvalid
        });
    }
    validate_content_range(&response, 0, 1, expected_size)?;
    let mut byte = [0_u8; 2];
    let count = response
        .read(&mut byte)
        .map_err(|_| Hdf5RangeTransferError::PartialResponse)?;
    if count != 1 {
        return Err(Hdf5RangeTransferError::PartialResponse);
    }
    if response
        .read(&mut byte[1..])
        .map_err(|_| Hdf5RangeTransferError::PartialResponse)?
        != 0
    {
        return Err(Hdf5RangeTransferError::ResponseInvalid);
    }
    let final_url = response.url().clone();
    let locator_hash = live_locator_hash(&final_url);
    Ok(NisarLivePreflightV1 {
        source_validator: NisarSubsetSourceValidatorV1 {
            kind: NisarSubsetValidatorKind::StrongEtag,
            value: etag.clone(),
            content_length: expected_size,
        },
        final_host_class: "earthdata_asf_v1",
        final_path_class: "approved_nisar_hdf5_object",
        locator_hash,
        validator_hash: sha256(etag.as_bytes()),
        request_count,
        response_body_bytes: 1,
    })
}

#[cfg(feature = "nisar-subset-production")]
#[doc(hidden)]
pub fn prefetch_live_nisar_metadata(
    http: &HttpTransfer,
    asset: &ResolvedAsset,
    workspace: &Hdf5RangeWorkspaceV1,
    source_validator: &NisarSubsetSourceValidatorV1,
    full_product_bytes: u64,
    cancelled: &(dyn Fn() -> bool + Sync),
) -> Result<NisarLiveMetadataPrefetchV1, Hdf5RangeTransferError> {
    prefetch_live_nisar_metadata_with_budget(
        http,
        asset,
        workspace,
        source_validator,
        full_product_bytes,
        cancelled,
        None,
    )
}

#[cfg(feature = "nisar-subset-production")]
#[doc(hidden)]
pub(crate) fn prefetch_live_nisar_metadata_with_budget(
    http: &HttpTransfer,
    asset: &ResolvedAsset,
    workspace: &Hdf5RangeWorkspaceV1,
    source_validator: &NisarSubsetSourceValidatorV1,
    full_product_bytes: u64,
    cancelled: &(dyn Fn() -> bool + Sync),
    budget: Option<&NisarLiveNetworkBudgetV1>,
) -> Result<NisarLiveMetadataPrefetchV1, Hdf5RangeTransferError> {
    if source_validator.kind != NisarSubsetValidatorKind::StrongEtag
        || source_validator.value.is_empty()
        || source_validator.content_length != full_product_bytes
        || full_product_bytes == 0
    {
        return Err(Hdf5RangeTransferError::StrongValidatorRequired);
    }
    if let Some(budget) = budget {
        budget.reserve_response_body_bytes(
            NISAR_LIVE_METADATA_PREFIX_BYTES
                .checked_add(1)
                .ok_or(Hdf5RangeTransferError::RangeBytesExceeded)?,
        )?;
    }
    let outcome = match budget {
        Some(budget) => http.send_resolved_exact_range_with_nisar_budget(
            asset,
            0,
            NISAR_LIVE_METADATA_PREFIX_BYTES,
            &source_validator.value,
            cancelled,
            budget,
        ),
        None => http.send_resolved_exact_range(
            asset,
            0,
            NISAR_LIVE_METADATA_PREFIX_BYTES,
            &source_validator.value,
            cancelled,
        ),
    };
    let (mut response, request_count) = match outcome {
        ExactRangeSendOutcome::Response {
            response,
            request_count,
        } => (response, request_count),
        ExactRangeSendOutcome::Cancelled { .. } => return Err(Hdf5RangeTransferError::Cancelled),
        ExactRangeSendOutcome::RetryableExhausted { .. }
        | ExactRangeSendOutcome::ProtocolRejected { .. }
        | ExactRangeSendOutcome::TrustedRedirectRejected { .. } => {
            return Err(Hdf5RangeTransferError::ResponseInvalid)
        }
        ExactRangeSendOutcome::BudgetExceeded { .. } => {
            return Err(Hdf5RangeTransferError::RangeCountExceeded)
        }
    };
    if matches!(
        response.status(),
        StatusCode::UNAUTHORIZED | StatusCode::FORBIDDEN
    ) {
        return Err(Hdf5RangeTransferError::CredentialsRequired);
    }
    let response_etag = response_strong_etag(&response)?;
    if response_etag != source_validator.value {
        return Err(Hdf5RangeTransferError::SourceChanged);
    }
    if response.status() != StatusCode::PARTIAL_CONTENT {
        return Err(if response.status() == StatusCode::OK {
            Hdf5RangeTransferError::RangeUnsupported
        } else {
            Hdf5RangeTransferError::ResponseInvalid
        });
    }
    validate_content_range(
        &response,
        0,
        NISAR_LIVE_METADATA_PREFIX_BYTES,
        full_product_bytes,
    )?;
    let _workspace_lock = workspace.acquire_lock()?;
    let (mut part, _identity) = open_part(&workspace.part_path(), full_product_bytes)?;
    let mut digest = Sha256::new();
    let mut received = 0_u64;
    let mut buffer = [0_u8; 64 * 1024];
    while received < NISAR_LIVE_METADATA_PREFIX_BYTES {
        if cancelled() {
            return Err(Hdf5RangeTransferError::Cancelled);
        }
        let remaining = NISAR_LIVE_METADATA_PREFIX_BYTES - received;
        let limit = usize::try_from(remaining.min(buffer.len() as u64))
            .map_err(|_| Hdf5RangeTransferError::RangeBytesExceeded)?;
        let count = response
            .read(&mut buffer[..limit])
            .map_err(|_| Hdf5RangeTransferError::PartialResponse)?;
        if count == 0 {
            return Err(Hdf5RangeTransferError::PartialResponse);
        }
        part.seek(SeekFrom::Start(received))
            .and_then(|_| part.write_all(&buffer[..count]))
            .map_err(|_| Hdf5RangeTransferError::Persistence)?;
        digest.update(&buffer[..count]);
        received = received
            .checked_add(count as u64)
            .ok_or(Hdf5RangeTransferError::RangeBytesExceeded)?;
    }
    if response
        .read(&mut buffer[..1])
        .map_err(|_| Hdf5RangeTransferError::PartialResponse)?
        != 0
    {
        return Err(Hdf5RangeTransferError::ResponseInvalid);
    }
    part.sync_data()
        .map_err(|_| Hdf5RangeTransferError::Persistence)?;
    Ok(NisarLiveMetadataPrefetchV1 {
        request_count,
        response_body_bytes: received,
        prefix_sha256: format!("{:x}", digest.finalize()),
    })
}

#[cfg(feature = "nisar-subset-production")]
#[doc(hidden)]
pub fn live_nisar_range_plan_from_sparse_hdf5(
    request: &NisarSubsetRequestManifestV1,
    sparse_path: &Path,
    metadata_evidence_sha256: &str,
) -> Result<Hdf5RangePlanV1, Hdf5RangeTransferError> {
    request
        .validate()
        .map_err(|_| Hdf5RangeTransferError::RangeInvalid)?;
    if !is_sha256(metadata_evidence_sha256) {
        return Err(Hdf5RangeTransferError::RangeInvalid);
    }
    let file =
        crate::hdf5_file::open(sparse_path).map_err(|_| Hdf5RangeTransferError::ResponseInvalid)?;
    let mut chunks = Vec::new();
    let mut datasets = BTreeSet::<(u16, String)>::new();
    for window in &request.mapping.windows {
        let component_id = window.component_id;
        let mut required = vec![(
            window.dataset_path.clone(),
            Hdf5ChunkDatatypeV1::ComplexF32LittleEndianRi,
        )];
        if window.validity_mask_required {
            required.push((
                validity_mask_dataset_path(&window.frequency),
                Hdf5ChunkDatatypeV1::UnsignedU8,
            ));
        }
        for (dataset_path, datatype) in required {
            if !datasets.insert((window.component_id, dataset_path.clone())) {
                continue;
            }
            let dataset = file
                .dataset(&dataset_path)
                .map_err(|_| Hdf5RangeTransferError::WindowIdentityMismatch)?;
            let shape = dataset.shape();
            let chunk_shape = dataset
                .dcpl()
                .map_err(|_| Hdf5RangeTransferError::RangeInvalid)?
                .get_chunk()
                .map_err(|_| Hdf5RangeTransferError::RangeInvalid)?
                .ok_or(Hdf5RangeTransferError::RangeInvalid)?;
            if shape.len() != 2 || chunk_shape.len() != 2 {
                return Err(Hdf5RangeTransferError::RangeInvalid);
            }
            let dataset_shape = [shape[0] as u64, shape[1] as u64];
            let chunk_shape = [chunk_shape[0] as u64, chunk_shape[1] as u64];
            let (datatype_sha256, fill_value_definition, fill_value_hex, filter_pipeline) =
                qualify_source_dataset_identity(&dataset, datatype)?;
            if dataset_shape.contains(&0)
                || chunk_shape.contains(&0)
                || dataset_shape[0] < window.line_end_exclusive
                || dataset_shape[1] < window.sample_end_exclusive
                || (dataset_path == window.dataset_path
                    && (dataset_shape != window.source_shape || chunk_shape != window.chunk_shape))
            {
                return Err(Hdf5RangeTransferError::WindowIdentityMismatch);
            }
            // Downloader enhancement D1d: one pass over the chunk index.
            crate::hdf5_chunk_visit::visit_stored_chunks(&dataset, |chunk| {
                if chunk.offset.len() != 2 {
                    return Err(Hdf5RangeTransferError::RangeInvalid);
                }
                let line_start = chunk.offset[0];
                let sample_start = chunk.offset[1];
                let line_end = line_start
                    .checked_add(chunk_shape[0])
                    .ok_or(Hdf5RangeTransferError::RangeInvalid)?;
                let sample_end = sample_start
                    .checked_add(chunk_shape[1])
                    .ok_or(Hdf5RangeTransferError::RangeInvalid)?;
                if line_start >= window.line_end_exclusive
                    || line_end <= window.line_start
                    || sample_start >= window.sample_end_exclusive
                    || sample_end <= window.sample_start
                {
                    return Ok(());
                }
                let end = chunk
                    .address
                    .checked_add(chunk.size)
                    .ok_or(Hdf5RangeTransferError::RangeBytesExceeded)?;
                if chunk.address == u64::MAX || chunk.filter_mask != 0 {
                    return Err(Hdf5RangeTransferError::RangeInvalid);
                }
                // Real product stores chunk payloads sparsely (physical
                // size < logical capacity); download exactly the stored size.
                chunks.push(Hdf5ChunkPhysicalRangeV1 {
                    dataset_path: dataset_path.clone(),
                    component_id,
                    logical_chunk: [line_start / chunk_shape[0], sample_start / chunk_shape[1]],
                    dataset_shape,
                    chunk_shape,
                    datatype,
                    datatype_sha256: datatype_sha256.clone(),
                    fill_value_definition,
                    fill_value_hex: fill_value_hex.clone(),
                    filter_pipeline,
                    allocation: Hdf5ChunkAllocationV1::Allocated {
                        filter_mask: chunk.filter_mask,
                        stored_size: chunk.size,
                        start: chunk.address,
                        end_exclusive: end,
                    },
                });
                Ok(())
            })
            .map_err(|error| match error {
                crate::hdf5_chunk_visit::ChunkVisitError::Ended(error) => error,
                _ => Hdf5RangeTransferError::RangeInvalid,
            })?;
        }
    }
    Hdf5RangePlanV1::from_evidence_with_metadata_prefix(
        request,
        ApprovedHdf5ChunkSetV1 {
            request_manifest_sha256: request
                .sha256()
                .map_err(|_| Hdf5RangeTransferError::RangeInvalid)?,
            source_length: request.expected_output.full_product_bytes,
            layout_evidence_digest:
                "86a615c78d3a9d1407ec3b5f4ddc76a10a5aff157b473bba48d9d31865e71500".to_owned(),
            metadata_evidence_sha256: metadata_evidence_sha256.to_owned(),
            chunks,
        },
        NISAR_LIVE_METADATA_PREFIX_BYTES,
    )
}

#[cfg(feature = "nisar-subset-production")]
#[doc(hidden)]
#[cfg(feature = "nisar-subset-production")]
#[doc(hidden)]
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct NisarLiveMetadataDiscoveryV1 {
    pub request_count: u64,
    pub response_body_bytes: u64,
    pub discovered_ranges: Vec<(u64, u64)>,
}

/// Physical payload ranges proven from real HDF5 object/chunk metadata for the
/// V2 planning-only bootstrap. This catalog never reads dataset values.
#[cfg(feature = "nisar-subset-production")]
pub(crate) struct NisarPlanningPhysicalRangeCatalogV2 {
    pub(crate) selected_science_ranges: Vec<(u64, u64)>,
    pub(crate) all_science_ranges: Vec<(u64, u64)>,
    pub(crate) metadata_ranges: Vec<(u64, u64)>,
    pub(crate) geolocation_ranges: Vec<(u64, u64)>,
}

#[cfg(feature = "nisar-subset-production")]
fn selected_science_frequency_label_v2(
    selected_science_paths: &[String],
    science_root: NisarScienceRoot,
) -> Result<&'static str, Hdf5RangeTransferError> {
    let swaths_path = science_root.swaths_path();
    let frequency_a_prefix = format!("{swaths_path}/frequencyA/");
    let frequency_b_prefix = format!("{swaths_path}/frequencyB/");
    let mut selected_frequency = None;
    for path in selected_science_paths {
        let frequency = if path.starts_with(&frequency_a_prefix) {
            "A"
        } else if path.starts_with(&frequency_b_prefix) {
            "B"
        } else {
            return Err(Hdf5RangeTransferError::RangeInvalid);
        };
        match selected_frequency {
            None => selected_frequency = Some(frequency),
            Some(existing) if existing == frequency => {}
            Some(_) => return Err(Hdf5RangeTransferError::RangeInvalid),
        }
    }
    selected_frequency.ok_or(Hdf5RangeTransferError::RangeInvalid)
}

#[cfg(feature = "nisar-subset-production")]
pub(crate) fn inspect_nisar_planning_physical_ranges_v2(
    source_path: &Path,
    selected_science_paths: &[String],
    science_root: NisarScienceRoot,
) -> Result<NisarPlanningPhysicalRangeCatalogV2, Hdf5RangeTransferError> {
    let geolocation_prefix = science_root.geolocation_path();
    let selected_frequency =
        selected_science_frequency_label_v2(selected_science_paths, science_root)?;

    let source_size = std::fs::metadata(source_path)
        .map_err(|_| Hdf5RangeTransferError::ResponseInvalid)?
        .len();
    if source_size == 0 || selected_science_paths.is_empty() {
        return Err(Hdf5RangeTransferError::RangeInvalid);
    }
    let file =
        crate::hdf5_file::open(source_path).map_err(|_| Hdf5RangeTransferError::ResponseInvalid)?;
    let mut selected_science_ranges = Vec::new();
    for path in selected_science_paths {
        let dataset = file
            .dataset(path)
            .map_err(|_| Hdf5RangeTransferError::RangeInvalid)?;
        push_dataset_ranges(&dataset, &mut selected_science_ranges)?;
    }
    let mut all_science_ranges = Vec::new();
    for frequency in ["A", "B"] {
        for polarization in ["HH", "HV", "VH", "VV", "LH", "LV", "RH", "RV"] {
            let path = science_root.selected_science_path(frequency, polarization);
            if file.link_exists(&path) {
                let dataset = file
                    .dataset(&path)
                    .map_err(|_| Hdf5RangeTransferError::RangeInvalid)?;
                push_dataset_ranges(&dataset, &mut all_science_ranges)?;
            }
        }
    }
    let mut all_metadata_ranges = Vec::new();
    match science_root {
        NisarScienceRoot::Lsar => {
            // Preserve the reviewed LSAR V2 phase boundary: source identity
            // values are planning inputs, while orbit/attitude/processing and
            // calibration payloads remain execution dependencies.
            let identification_prefix = science_root.identification_path();
            let identification = file
                .group(&identification_prefix)
                .map_err(|_| Hdf5RangeTransferError::RangeInvalid)?;
            collect_metadata_ranges_inner(
                &file,
                &identification,
                &identification_prefix,
                &mut all_metadata_ranges,
                science_root,
            )?;
        }
        NisarScienceRoot::Ssar => {
            // Keep the SSAR path exactly aligned with the real r27 closure:
            // its fragmented provider product needs the reviewed metadata
            // payload set in addition to the explicit geolocation/radar axes.
            collect_metadata_ranges_for_science_root(
                &file,
                "/",
                &mut all_metadata_ranges,
                science_root,
            )?;
        }
    }
    let geolocation = file
        .group(&geolocation_prefix)
        .map_err(|_| Hdf5RangeTransferError::RangeInvalid)?;
    let mut geolocation_ranges = Vec::new();
    collect_metadata_ranges_inner(
        &file,
        &geolocation,
        &geolocation_prefix,
        &mut geolocation_ranges,
        science_root,
    )?;
    for path in [
        science_root.zero_doppler_time_path(),
        science_root.slant_range_path(selected_frequency),
    ] {
        let dataset = file
            .dataset(&path)
            .map_err(|_| Hdf5RangeTransferError::RangeInvalid)?;
        push_dataset_ranges(&dataset, &mut geolocation_ranges)?;
    }
    let selected_science_ranges = canonical_planning_ranges(selected_science_ranges, source_size)?;
    let all_science_ranges = canonical_planning_ranges(all_science_ranges, source_size)?;
    let geolocation_ranges = canonical_planning_ranges(geolocation_ranges, source_size)?;
    let metadata_ranges = canonical_planning_ranges(
        all_metadata_ranges
            .into_iter()
            .filter(|range| {
                !geolocation_ranges
                    .iter()
                    .any(|geolocation| planning_ranges_overlap(*range, *geolocation))
            })
            .collect(),
        source_size,
    )?;
    Ok(NisarPlanningPhysicalRangeCatalogV2 {
        selected_science_ranges,
        all_science_ranges,
        metadata_ranges,
        geolocation_ranges,
    })
}

#[cfg(feature = "nisar-subset-production")]
fn canonical_planning_ranges(
    mut ranges: Vec<(u64, u64)>,
    source_size: u64,
) -> Result<Vec<(u64, u64)>, Hdf5RangeTransferError> {
    if ranges
        .iter()
        .any(|(start, end)| start >= end || *end > source_size)
    {
        return Err(Hdf5RangeTransferError::RangeInvalid);
    }
    ranges.sort_unstable();
    ranges.dedup();
    let mut canonical = Vec::<(u64, u64)>::new();
    for range in ranges {
        if let Some(last) = canonical.last_mut() {
            if range.0 <= last.1 {
                last.1 = last.1.max(range.1);
                continue;
            }
        }
        canonical.push(range);
    }
    Ok(canonical)
}

#[cfg(feature = "nisar-subset-production")]
fn planning_ranges_overlap(left: (u64, u64), right: (u64, u64)) -> bool {
    left.0 < right.1 && right.0 < left.1
}

/// Downloads the physical ranges that hold the real product's metadata
/// dataset values outside the initial contiguous prefix.
///
/// The 195 MiB prefix materializes every object header and link (207
/// datasets are enumerable), but a few metadata payloads (RFI tables, some
/// geolocation-grid axes) are physically stored past the prefix. This
/// function discovers those ranges structurally from the HDF5 chunk index /
/// storage offsets (never a hard-coded granule offset), then fetches only
/// the missing bytes with the frozen strong validator (If-Range, 206,
/// exact Content-Range). Science swath payloads are intentionally not
/// downloaded here; they are planned and fetched by the science Range
/// transfer.
#[cfg(feature = "nisar-subset-production")]
#[doc(hidden)]
pub fn discover_live_nisar_metadata_chunks(
    http: &HttpTransfer,
    asset: &ResolvedAsset,
    workspace: &Hdf5RangeWorkspaceV1,
    source_validator: &NisarSubsetSourceValidatorV1,
    full_product_bytes: u64,
    cancelled: &(dyn Fn() -> bool + Sync),
) -> Result<NisarLiveMetadataDiscoveryV1, Hdf5RangeTransferError> {
    discover_live_nisar_metadata_chunks_with_budget(
        http,
        asset,
        workspace,
        source_validator,
        full_product_bytes,
        cancelled,
        None,
    )
}

#[cfg(feature = "nisar-subset-production")]
#[doc(hidden)]
pub(crate) fn discover_live_nisar_metadata_chunks_with_budget(
    http: &HttpTransfer,
    asset: &ResolvedAsset,
    workspace: &Hdf5RangeWorkspaceV1,
    source_validator: &NisarSubsetSourceValidatorV1,
    full_product_bytes: u64,
    cancelled: &(dyn Fn() -> bool + Sync),
    budget: Option<&NisarLiveNetworkBudgetV1>,
) -> Result<NisarLiveMetadataDiscoveryV1, Hdf5RangeTransferError> {
    if source_validator.kind != NisarSubsetValidatorKind::StrongEtag
        || source_validator.value.is_empty()
        || source_validator.content_length != full_product_bytes
        || full_product_bytes == 0
    {
        return Err(Hdf5RangeTransferError::StrongValidatorRequired);
    }
    let _workspace_lock = workspace.acquire_lock()?;
    // Read-only HDF5 open first so no write handle is held at the same time
    // (Windows rejects a second open of the same sparse part while a write
    // handle is live).
    let mut ranges = {
        let file = crate::hdf5_file::open(workspace.part_path())
            .map_err(|_| Hdf5RangeTransferError::ResponseInvalid)?;
        let mut ranges = Vec::<(u64, u64)>::new();
        collect_metadata_ranges(&file, "/", &mut ranges)?;
        ranges
    };
    let (mut part, _identity) = open_part(&workspace.part_path(), full_product_bytes)?;
    if ranges.is_empty() {
        return Ok(NisarLiveMetadataDiscoveryV1 {
            request_count: 0,
            response_body_bytes: 0,
            discovered_ranges: Vec::new(),
        });
    }
    ranges.sort_unstable_by_key(|range| range.0);
    let mut merged = Vec::<(u64, u64)>::new();
    for (start, end) in ranges {
        if end <= start {
            continue;
        }
        if let Some(last) = merged.last_mut() {
            if start <= last.1 {
                last.1 = last.1.max(end);
                continue;
            }
        }
        merged.push((start, end));
    }
    let mut request_count = 0_u64;
    let mut response_body_bytes = 0_u64;
    let mut downloaded = Vec::<(u64, u64)>::new();
    for (start, end) in merged {
        // Skip bytes already inside the materialized prefix.
        if end <= NISAR_LIVE_METADATA_PREFIX_BYTES {
            continue;
        }
        let fetch_start = start.max(NISAR_LIVE_METADATA_PREFIX_BYTES);
        if fetch_start >= end {
            continue;
        }
        if let Some(budget) = budget {
            budget.reserve_response_body_bytes(
                end.checked_sub(fetch_start)
                    .and_then(|bytes| bytes.checked_add(1))
                    .ok_or(Hdf5RangeTransferError::RangeBytesExceeded)?,
            )?;
        }
        let outcome = match budget {
            Some(budget) => http.send_resolved_exact_range_with_nisar_budget(
                asset,
                fetch_start,
                end,
                &source_validator.value,
                cancelled,
                budget,
            ),
            None => http.send_resolved_exact_range(
                asset,
                fetch_start,
                end,
                &source_validator.value,
                cancelled,
            ),
        };
        let (mut response, count) = match outcome {
            ExactRangeSendOutcome::Response {
                response,
                request_count,
            } => (response, request_count),
            ExactRangeSendOutcome::Cancelled { .. } => {
                return Err(Hdf5RangeTransferError::Cancelled)
            }
            ExactRangeSendOutcome::RetryableExhausted { .. }
            | ExactRangeSendOutcome::ProtocolRejected { .. }
            | ExactRangeSendOutcome::TrustedRedirectRejected { .. } => {
                return Err(Hdf5RangeTransferError::ResponseInvalid)
            }
            ExactRangeSendOutcome::BudgetExceeded { .. } => {
                return Err(Hdf5RangeTransferError::RangeCountExceeded)
            }
        };
        if matches!(
            response.status(),
            StatusCode::UNAUTHORIZED | StatusCode::FORBIDDEN
        ) {
            return Err(Hdf5RangeTransferError::CredentialsRequired);
        }
        let response_etag = response_strong_etag(&response)?;
        if response_etag != source_validator.value {
            return Err(Hdf5RangeTransferError::SourceChanged);
        }
        if response.status() != StatusCode::PARTIAL_CONTENT {
            return Err(if response.status() == StatusCode::OK {
                Hdf5RangeTransferError::RangeUnsupported
            } else {
                Hdf5RangeTransferError::ResponseInvalid
            });
        }
        validate_content_range(&response, fetch_start, end, full_product_bytes)?;
        let mut received = 0_u64;
        let mut buffer = [0_u8; 64 * 1024];
        part.seek(SeekFrom::Start(fetch_start))
            .map_err(|_| Hdf5RangeTransferError::Persistence)?;
        while received < end - fetch_start {
            if cancelled() {
                return Err(Hdf5RangeTransferError::Cancelled);
            }
            let remaining = end - fetch_start - received;
            let limit = usize::try_from(remaining.min(buffer.len() as u64))
                .map_err(|_| Hdf5RangeTransferError::RangeBytesExceeded)?;
            let count = response
                .read(&mut buffer[..limit])
                .map_err(|_| Hdf5RangeTransferError::PartialResponse)?;
            if count == 0 {
                return Err(Hdf5RangeTransferError::PartialResponse);
            }
            part.write_all(&buffer[..count])
                .map_err(|_| Hdf5RangeTransferError::Persistence)?;
            received += count as u64;
        }
        if response
            .read(&mut buffer[..1])
            .map_err(|_| Hdf5RangeTransferError::PartialResponse)?
            != 0
        {
            return Err(Hdf5RangeTransferError::ResponseInvalid);
        }
        part.sync_all()
            .map_err(|_| Hdf5RangeTransferError::Persistence)?;
        request_count = request_count
            .checked_add(count)
            .ok_or(Hdf5RangeTransferError::RangeCountExceeded)?;
        response_body_bytes = response_body_bytes
            .checked_add(end - fetch_start)
            .ok_or(Hdf5RangeTransferError::RangeBytesExceeded)?;
        downloaded.push((fetch_start, end));
    }
    Ok(NisarLiveMetadataDiscoveryV1 {
        request_count,
        response_body_bytes,
        discovered_ranges: downloaded,
    })
}

#[cfg(feature = "nisar-subset-production")]
fn is_metadata_payload_path(path: &str, science_root: NisarScienceRoot) -> bool {
    path.starts_with(&format!("{}/", science_root.metadata_path()))
        || path.starts_with(&format!("{}/", science_root.identification_path()))
}

#[cfg(feature = "nisar-subset-production")]
fn collect_metadata_ranges(
    file: &hdf5::File,
    prefix: &str,
    ranges: &mut Vec<(u64, u64)>,
) -> Result<(), Hdf5RangeTransferError> {
    collect_metadata_ranges_for_science_root(file, prefix, ranges, NisarScienceRoot::Lsar)
}

#[cfg(feature = "nisar-subset-production")]
fn collect_metadata_ranges_for_science_root(
    file: &hdf5::File,
    prefix: &str,
    ranges: &mut Vec<(u64, u64)>,
    science_root: NisarScienceRoot,
) -> Result<(), Hdf5RangeTransferError> {
    collect_metadata_ranges_inner(file, file, prefix, ranges, science_root)
}

#[cfg(feature = "nisar-subset-production")]
fn collect_metadata_ranges_inner(
    file: &hdf5::File,
    group: &hdf5::Group,
    prefix: &str,
    ranges: &mut Vec<(u64, u64)>,
    science_root: NisarScienceRoot,
) -> Result<(), Hdf5RangeTransferError> {
    let names = group
        .member_names()
        .map_err(|_| Hdf5RangeTransferError::ResponseInvalid)?;
    for name in names {
        let child = if prefix == "/" {
            format!("/{name}")
        } else {
            format!("{prefix}/{name}")
        };
        match group.dataset(&name) {
            Ok(dataset) => {
                if is_metadata_payload_path(&child, science_root) {
                    push_dataset_ranges(&dataset, ranges)?;
                }
            }
            Err(_) => match group.group(&name) {
                Ok(sub) => {
                    collect_metadata_ranges_inner(file, &sub, &child, ranges, science_root)?;
                }
                Err(_) if planning_named_datatype_exists(group, &name)? => {}
                Err(_) => return Err(Hdf5RangeTransferError::ResponseInvalid),
            },
        }
    }
    let _ = file;
    Ok(())
}

#[cfg(feature = "nisar-subset-production")]
fn planning_named_datatype_exists(
    group: &hdf5::Group,
    name: &str,
) -> Result<bool, Hdf5RangeTransferError> {
    let name = CString::new(name).map_err(|_| Hdf5RangeTransferError::ResponseInvalid)?;
    let datatype_id =
        hdf5::sync::sync(|| unsafe { H5Topen2(group.id(), name.as_ptr(), H5P_DEFAULT) });
    if datatype_id < 0 {
        return Ok(false);
    }
    let close_status = hdf5::sync::sync(|| unsafe { H5Tclose(datatype_id) });
    if close_status < 0 {
        return Err(Hdf5RangeTransferError::ResponseInvalid);
    }
    Ok(true)
}

#[cfg(feature = "nisar-subset-production")]
fn push_dataset_ranges(
    dataset: &hdf5::Dataset,
    ranges: &mut Vec<(u64, u64)>,
) -> Result<(), Hdf5RangeTransferError> {
    // Downloader enhancement D1d: one pass over the chunk index. A chunked
    // dataset contributes its stored chunks (none when nothing is stored); an
    // iteration failure fails closed.
    match crate::hdf5_chunk_visit::visit_stored_chunks(dataset, |chunk| {
        if chunk.size > 0 {
            ranges.push((chunk.address, chunk.address.saturating_add(chunk.size)));
        }
        Ok::<(), Hdf5RangeTransferError>(())
    }) {
        Ok(_) => {}
        Err(crate::hdf5_chunk_visit::ChunkVisitError::Ended(error)) => return Err(error),
        Err(crate::hdf5_chunk_visit::ChunkVisitError::Hdf5) => {
            return Err(Hdf5RangeTransferError::ResponseInvalid)
        }
        Err(crate::hdf5_chunk_visit::ChunkVisitError::NotChunked) => {
            let offset = hdf5::sync::sync(|| unsafe { hdf5_sys::h5d::H5Dget_offset(dataset.id()) });
            let storage =
                hdf5::sync::sync(|| unsafe { hdf5_sys::h5d::H5Dget_storage_size(dataset.id()) });
            if storage > 0 {
                ranges.push((offset, offset.saturating_add(storage)));
            }
        }
    }
    Ok(())
}

#[cfg(feature = "nisar-subset-production")]
#[doc(hidden)]
pub fn seed_live_metadata_range(
    workspace: &Hdf5RangeWorkspaceV1,
    request: &NisarSubsetRequestManifestV1,
    plan: &Hdf5RangePlanV1,
    source_validator: &NisarSubsetSourceValidatorV1,
    prefix_sha256: &str,
    timestamp: &str,
) -> Result<(), Hdf5RangeTransferError> {
    request
        .validate()
        .map_err(|_| Hdf5RangeTransferError::RangeInvalid)?;
    let request_sha256 = request
        .sha256()
        .map_err(|_| Hdf5RangeTransferError::RangeInvalid)?;
    if plan.metadata_prefix_bytes != NISAR_LIVE_METADATA_PREFIX_BYTES
        || plan.ranges.first().is_none_or(|range| {
            range.start != 0
                || range.end_exclusive != NISAR_LIVE_METADATA_PREFIX_BYTES
                || !range.member_chunk_indices.is_empty()
        })
        || source_validator.kind != NisarSubsetValidatorKind::StrongEtag
        || source_validator.content_length != request.expected_output.full_product_bytes
        || !is_sha256(prefix_sha256)
    {
        return Err(Hdf5RangeTransferError::WindowIdentityMismatch);
    }
    if plan.ranges.len() < 2 || plan.request_manifest_sha256 != request_sha256 {
        return Err(Hdf5RangeTransferError::WindowIdentityMismatch);
    }
    let _workspace_lock = workspace.acquire_lock()?;
    if workspace.load_manifest()?.is_some() {
        return Err(Hdf5RangeTransferError::RecoveryInvalid);
    }
    let (mut part, identity) = open_part(
        &workspace.part_path(),
        request.expected_output.full_product_bytes,
    )?;
    workspace.bind_part_identity(plan, &identity)?;
    let actual_prefix_sha256 = hash_file_range(&mut part, 0, NISAR_LIVE_METADATA_PREFIX_BYTES)?;
    if actual_prefix_sha256 != prefix_sha256 {
        return Err(Hdf5RangeTransferError::SourceChanged);
    }
    let etag = source_validator.value.as_str();
    let mut manifest = planned_manifest(request, plan, timestamp, etag)?;
    manifest.ranges[0].received_bytes = NISAR_LIVE_METADATA_PREFIX_BYTES;
    manifest.ranges[0].sha256 = Some(prefix_sha256.to_owned());
    manifest.ranges[0].state = NisarSubsetRangeState::Complete;
    manifest.received_bytes = NISAR_LIVE_METADATA_PREFIX_BYTES;
    manifest.state = NisarSubsetTransferState::InProgress;
    manifest.updated_at = timestamp.to_owned();
    manifest
        .validate()
        .map_err(|_| Hdf5RangeTransferError::Persistence)?;
    persist_manifest(workspace, &manifest)?;
    workspace.persist_metrics(
        plan,
        Hdf5RangeTransferMetricsV1 {
            request_count: 0,
            response_body_bytes: NISAR_LIVE_METADATA_PREFIX_BYTES,
            accepted_unique_bytes: NISAR_LIVE_METADATA_PREFIX_BYTES,
            retry_duplicate_bytes: 0,
        },
    )?;
    Ok(())
}

#[cfg(feature = "nisar-subset-production")]
fn hash_file_range(
    file: &mut File,
    start: u64,
    end_exclusive: u64,
) -> Result<String, Hdf5RangeTransferError> {
    file.seek(SeekFrom::Start(start))
        .map_err(|_| Hdf5RangeTransferError::RecoveryInvalid)?;
    let mut digest = Sha256::new();
    let mut remaining = end_exclusive
        .checked_sub(start)
        .ok_or(Hdf5RangeTransferError::RangeInvalid)?;
    let mut buffer = [0_u8; 64 * 1024];
    while remaining > 0 {
        let count = usize::try_from(remaining.min(buffer.len() as u64))
            .map_err(|_| Hdf5RangeTransferError::RangeBytesExceeded)?;
        file.read_exact(&mut buffer[..count])
            .map_err(|_| Hdf5RangeTransferError::RecoveryInvalid)?;
        digest.update(&buffer[..count]);
        remaining -= count as u64;
    }
    Ok(format!("{:x}", digest.finalize()))
}

#[cfg(feature = "nisar-subset-production")]
fn live_locator_hash(url: &reqwest::Url) -> String {
    let mut identity = String::new();
    identity.push_str(url.scheme());
    identity.push_str("://");
    identity.push_str(url.host_str().unwrap_or_default());
    identity.push_str(url.path());
    sha256(identity.as_bytes())
}

fn same_component_agnostic_chunk(
    left: &Hdf5ChunkPhysicalRangeV1,
    right: &Hdf5ChunkPhysicalRangeV1,
) -> bool {
    left.dataset_path == right.dataset_path
        && left.logical_chunk == right.logical_chunk
        && left.dataset_shape == right.dataset_shape
        && left.chunk_shape == right.chunk_shape
        && left.datatype == right.datatype
        && left.datatype_sha256 == right.datatype_sha256
        && left.fill_value_definition == right.fill_value_definition
        && left.fill_value_hex == right.fill_value_hex
        && left.filter_pipeline == right.filter_pipeline
        && left.allocation == right.allocation
}

impl Hdf5PhysicalRangeV1 {
    pub fn member_chunk_indices(&self) -> &[u32] {
        &self.member_chunk_indices
    }
}

fn compute_plan_sha256(plan: &Hdf5RangePlanV1) -> Result<String, Hdf5RangeTransferError> {
    #[derive(Serialize)]
    struct PlanIdentity<'a> {
        plan_contract: &'a str,
        source_length: u64,
        metadata_prefix_bytes: u64,
        chunks: &'a [Hdf5ChunkPhysicalRangeV1],
        ranges: &'a [Hdf5PhysicalRangeV1],
        expected_remote_bytes: u64,
        request_manifest_sha256: &'a str,
        layout_evidence_digest: &'a str,
        metadata_evidence_sha256: &'a str,
    }
    let canonical = serde_json::to_vec(&PlanIdentity {
        plan_contract: &plan.plan_contract,
        source_length: plan.source_length,
        metadata_prefix_bytes: plan.metadata_prefix_bytes,
        chunks: &plan.chunks,
        ranges: &plan.ranges,
        expected_remote_bytes: plan.expected_remote_bytes,
        request_manifest_sha256: &plan.request_manifest_sha256,
        layout_evidence_digest: &plan.layout_evidence_digest,
        metadata_evidence_sha256: &plan.metadata_evidence_sha256,
    })
    .map_err(|_| Hdf5RangeTransferError::RangeInvalid)?;
    Ok(sha256(&canonical))
}

fn is_sha256(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
}

fn validate_chunk_dataset_identity(
    request: &NisarSubsetRequestManifestV1,
    chunk: &Hdf5ChunkPhysicalRangeV1,
) -> Result<(), Hdf5RangeTransferError> {
    let expected_datatype_sha256 = supported_datatype_sha256(chunk.datatype)?;
    let pipeline_accepted = match chunk.filter_pipeline {
        Hdf5ChunkFilterPipelineV1::None => true,
        Hdf5ChunkFilterPipelineV1::ShuffleDeflate { level } => level == 4,
    };
    if !is_sha256(&chunk.datatype_sha256)
        || chunk.datatype_sha256 != expected_datatype_sha256
        || !pipeline_accepted
        || chunk.fill_value_hex.as_ref().is_some_and(|value| {
            value.len() != chunk.datatype.element_size() * 2
                || !value
                    .bytes()
                    .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
        })
        || matches!(
            chunk.fill_value_definition,
            Hdf5FillValueDefinitionV1::Undefined
        ) != chunk.fill_value_hex.is_none()
    {
        return Err(Hdf5RangeTransferError::RangeInvalid);
    }
    let expected_kind = if request.mapping.windows.iter().any(|window| {
        window.component_id == chunk.component_id && window.dataset_path == chunk.dataset_path
    }) {
        Hdf5ChunkDatatypeV1::ComplexF32LittleEndianRi
    } else {
        Hdf5ChunkDatatypeV1::UnsignedU8
    };
    if chunk.datatype != expected_kind {
        return Err(Hdf5RangeTransferError::RangeInvalid);
    }
    Ok(())
}

pub(crate) fn qualify_source_dataset_identity(
    dataset: &Dataset,
    expected_kind: Hdf5ChunkDatatypeV1,
) -> Result<
    (
        String,
        Hdf5FillValueDefinitionV1,
        Option<String>,
        Hdf5ChunkFilterPipelineV1,
    ),
    Hdf5RangeTransferError,
> {
    if cfg!(target_endian = "big")
        || dataset
            .dcpl()
            .and_then(|dcpl| dcpl.get_external())
            .map_or(true, |external| !external.is_empty())
    {
        return Err(Hdf5RangeTransferError::RangeInvalid);
    }
    // The admitted P05023 / spec-1.5.0 science and mask datasets use the
    // built-in Shuffle -> Deflate(4) pipeline. Only that exact reviewed
    // pipeline is accepted; unknown/arbitrary filters fail closed.
    let filter_pipeline = match dataset.filters().as_slice() {
        [] => Hdf5ChunkFilterPipelineV1::None,
        [Filter::Shuffle, Filter::Deflate(4)] => {
            Hdf5ChunkFilterPipelineV1::ShuffleDeflate { level: 4 }
        }
        _ => return Err(Hdf5RangeTransferError::RangeInvalid),
    };
    let actual = dataset
        .dtype()
        .map_err(|_| Hdf5RangeTransferError::RangeInvalid)?;
    let expected = match expected_kind {
        Hdf5ChunkDatatypeV1::ComplexF32LittleEndianRi => {
            Datatype::from_descriptor(&TypeDescriptor::Compound(CompoundType {
                fields: vec![
                    CompoundField::new("r", TypeDescriptor::Float(FloatSize::U4), 0, 0),
                    CompoundField::new("i", TypeDescriptor::Float(FloatSize::U4), 4, 1),
                ],
                size: 8,
            }))
        }
        Hdf5ChunkDatatypeV1::UnsignedU8 => Datatype::from_type::<u8>(),
    }
    .map_err(|_| Hdf5RangeTransferError::RangeInvalid)?;
    if actual != expected || actual.size() != expected_kind.element_size() {
        return Err(Hdf5RangeTransferError::RangeInvalid);
    }
    // Bind the plan to the one canonical encoded representation of each
    // supported datatype. The source datatype was already proven equal above;
    // hashing a separately opened dataset datatype can retain HDF5-internal
    // encoding details that are not part of the reviewed logical type.
    let datatype_sha256 = hdf5_datatype_sha256(&expected)?;
    let (definition, fill_value_hex) = fill_value_identity(dataset, &actual)?;
    Ok((datatype_sha256, definition, fill_value_hex, filter_pipeline))
}

pub(crate) fn supported_datatype_sha256(
    kind: Hdf5ChunkDatatypeV1,
) -> Result<String, Hdf5RangeTransferError> {
    if cfg!(target_endian = "big") {
        return Err(Hdf5RangeTransferError::RangeInvalid);
    }
    let datatype = match kind {
        Hdf5ChunkDatatypeV1::ComplexF32LittleEndianRi => {
            Datatype::from_descriptor(&TypeDescriptor::Compound(CompoundType {
                fields: vec![
                    CompoundField::new("r", TypeDescriptor::Float(FloatSize::U4), 0, 0),
                    CompoundField::new("i", TypeDescriptor::Float(FloatSize::U4), 4, 1),
                ],
                size: 8,
            }))
        }
        Hdf5ChunkDatatypeV1::UnsignedU8 => Datatype::from_type::<u8>(),
    }
    .map_err(|_| Hdf5RangeTransferError::RangeInvalid)?;
    hdf5_datatype_sha256(&datatype)
}

pub(crate) fn hdf5_datatype_sha256(datatype: &Datatype) -> Result<String, Hdf5RangeTransferError> {
    const MAX_IDENTITY_BYTES: usize = 64 * 1024;
    let mut encoded_len = 0_usize;
    let size_status = hdf5::sync::sync(|| unsafe {
        H5Tencode(datatype.id(), std::ptr::null_mut(), &mut encoded_len)
    });
    if size_status < 0 || encoded_len == 0 || encoded_len > MAX_IDENTITY_BYTES {
        return Err(Hdf5RangeTransferError::RangeInvalid);
    }
    let mut encoded = vec![0_u8; encoded_len];
    let mut written_len = encoded_len;
    let status = hdf5::sync::sync(|| unsafe {
        H5Tencode(datatype.id(), encoded.as_mut_ptr().cast(), &mut written_len)
    });
    if status < 0 || written_len != encoded_len {
        return Err(Hdf5RangeTransferError::RangeInvalid);
    }
    Ok(sha256(&encoded))
}

fn fill_value_identity(
    dataset: &Dataset,
    datatype: &Datatype,
) -> Result<(Hdf5FillValueDefinitionV1, Option<String>), Hdf5RangeTransferError> {
    let dcpl = dataset
        .dcpl()
        .map_err(|_| Hdf5RangeTransferError::RangeInvalid)?;
    let mut raw_definition = H5D_FILL_VALUE_ERROR;
    let status =
        hdf5::sync::sync(|| unsafe { H5Pfill_value_defined(dcpl.id(), &mut raw_definition) });
    if status < 0 {
        return Err(Hdf5RangeTransferError::RangeInvalid);
    }
    let definition = match raw_definition {
        H5D_FILL_VALUE_UNDEFINED => Hdf5FillValueDefinitionV1::Undefined,
        H5D_FILL_VALUE_DEFAULT => Hdf5FillValueDefinitionV1::Default,
        H5D_FILL_VALUE_USER_DEFINED => Hdf5FillValueDefinitionV1::UserDefined,
        _ => return Err(Hdf5RangeTransferError::RangeInvalid),
    };
    if definition == Hdf5FillValueDefinitionV1::Undefined {
        return Ok((definition, None));
    }
    let mut bytes = vec![0_u8; datatype.size()];
    let status = hdf5::sync::sync(|| unsafe {
        H5Pget_fill_value(dcpl.id(), datatype.id(), bytes.as_mut_ptr().cast())
    });
    if status < 0 {
        return Err(Hdf5RangeTransferError::RangeInvalid);
    }
    Ok((definition, Some(hex_bytes(&bytes))))
}

fn hex_bytes(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut value = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        value.push(HEX[(byte >> 4) as usize] as char);
        value.push(HEX[(byte & 0x0f) as usize] as char);
    }
    value
}

fn chunk_maps_to_request(
    request: &NisarSubsetRequestManifestV1,
    chunk: &Hdf5ChunkPhysicalRangeV1,
) -> bool {
    request.mapping.windows.iter().any(|window| {
        if window.component_id != chunk.component_id {
            return false;
        }
        let science = window.dataset_path == chunk.dataset_path;
        let mask = window.validity_mask_required
            && validity_mask_dataset_path(&window.frequency) == chunk.dataset_path;
        if !science && !mask {
            return false;
        }
        if science
            && (chunk.dataset_shape != window.source_shape
                || chunk.chunk_shape != window.chunk_shape)
        {
            return false;
        }
        if chunk.chunk_shape.contains(&0)
            || chunk.dataset_shape.contains(&0)
            || window.line_end_exclusive > chunk.dataset_shape[0]
            || window.sample_end_exclusive > chunk.dataset_shape[1]
        {
            return false;
        }
        let Some(line_start) = chunk.logical_chunk[0].checked_mul(chunk.chunk_shape[0]) else {
            return false;
        };
        let Some(sample_start) = chunk.logical_chunk[1].checked_mul(chunk.chunk_shape[1]) else {
            return false;
        };
        let Some(line_end) = line_start.checked_add(chunk.chunk_shape[0]) else {
            return false;
        };
        let Some(sample_end) = sample_start.checked_add(chunk.chunk_shape[1]) else {
            return false;
        };
        line_start < window.line_end_exclusive
            && line_end > window.line_start
            && sample_start < window.sample_end_exclusive
            && sample_end > window.sample_start
    })
}

fn validity_mask_dataset_path(frequency: &NisarSubsetFrequency) -> String {
    let frequency = match frequency {
        NisarSubsetFrequency::A => "A",
        NisarSubsetFrequency::B => "B",
    };
    format!("/science/LSAR/RSLC/swaths/frequency{frequency}/inputDataExceptionMask")
}

fn validate_science_chunk_coverage(
    request: &NisarSubsetRequestManifestV1,
    chunks: &[Hdf5ChunkPhysicalRangeV1],
) -> Result<(), Hdf5RangeTransferError> {
    let mut expected = BTreeSet::new();
    for window in &request.mapping.windows {
        insert_expected_chunks(
            &mut expected,
            window.component_id,
            &window.dataset_path,
            window.line_start,
            window.line_end_exclusive,
            window.sample_start,
            window.sample_end_exclusive,
            window.chunk_shape,
        )?;
        if window.validity_mask_required {
            let mask_path = validity_mask_dataset_path(&window.frequency);
            let mask_layout = chunks
                .iter()
                .find(|chunk| {
                    chunk.component_id == window.component_id && chunk.dataset_path == mask_path
                })
                .map(|chunk| (chunk.dataset_shape, chunk.chunk_shape))
                .ok_or(Hdf5RangeTransferError::WindowIdentityMismatch)?;
            if chunks.iter().any(|chunk| {
                chunk.component_id == window.component_id
                    && chunk.dataset_path == mask_path
                    && (chunk.dataset_shape, chunk.chunk_shape) != mask_layout
            }) {
                return Err(Hdf5RangeTransferError::WindowIdentityMismatch);
            }
            insert_expected_chunks(
                &mut expected,
                window.component_id,
                &mask_path,
                window.line_start,
                window.line_end_exclusive,
                window.sample_start,
                window.sample_end_exclusive,
                mask_layout.1,
            )?;
        }
    }
    let actual = chunks
        .iter()
        .map(|chunk| {
            (
                chunk.component_id,
                chunk.dataset_path.clone(),
                chunk.logical_chunk,
            )
        })
        .collect::<BTreeSet<_>>();
    if actual != expected {
        return Err(Hdf5RangeTransferError::WindowIdentityMismatch);
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn insert_expected_chunks(
    expected: &mut BTreeSet<(u16, String, [u64; 2])>,
    component_id: u16,
    dataset_path: &str,
    line_start: u64,
    line_end_exclusive: u64,
    sample_start: u64,
    sample_end_exclusive: u64,
    chunk_shape: [u64; 2],
) -> Result<(), Hdf5RangeTransferError> {
    let line_first = line_start / chunk_shape[0];
    let line_last = (line_end_exclusive - 1) / chunk_shape[0];
    let sample_first = sample_start / chunk_shape[1];
    let sample_last = (sample_end_exclusive - 1) / chunk_shape[1];
    for line in line_first..=line_last {
        for sample in sample_first..=sample_last {
            expected.insert((component_id, dataset_path.to_owned(), [line, sample]));
            if expected.len() > HDF5_RANGE_MAX_COUNT {
                return Err(Hdf5RangeTransferError::RangeCountExceeded);
            }
        }
    }
    Ok(())
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct NisarPlanningPartEvidenceV2 {
    pub(crate) identity_sha256: String,
    pub(crate) material_sha256: String,
}

pub(crate) struct ImmutableSciencePartForWriterV2 {
    source: File,
    view: File,
    view_path: PathBuf,
    source_identity_sha256: String,
    source_mutation_marker_sha256: String,
}

impl ImmutableSciencePartForWriterV2 {
    pub(crate) fn source(&self) -> &File {
        &self.source
    }

    pub(crate) fn view(&self) -> &File {
        &self.view
    }

    pub(crate) fn view_path(&self) -> &Path {
        &self.view_path
    }

    pub(crate) fn source_identity_sha256(&self) -> &str {
        &self.source_identity_sha256
    }

    pub(crate) fn source_mutation_marker_sha256(&self) -> &str {
        &self.source_mutation_marker_sha256
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Hdf5RangeWorkspaceV1 {
    root: Arc<PinnedDirectory>,
}

#[cfg(windows)]
fn mark_part_file_sparse(file: &File) -> Result<(), Hdf5RangeTransferError> {
    use std::os::windows::io::AsRawHandle;
    use windows_sys::Win32::System::Ioctl::FSCTL_SET_SPARSE;
    use windows_sys::Win32::System::IO::DeviceIoControl;

    let mut returned = 0_u32;
    let sparse = unsafe {
        DeviceIoControl(
            file.as_raw_handle(),
            FSCTL_SET_SPARSE,
            std::ptr::null(),
            0,
            std::ptr::null_mut(),
            0,
            &mut returned,
            std::ptr::null_mut(),
        )
    };
    if sparse == 0 {
        return Err(Hdf5RangeTransferError::Persistence);
    }
    Ok(())
}

#[cfg(not(windows))]
fn mark_part_file_sparse(_file: &File) -> Result<(), Hdf5RangeTransferError> {
    Ok(())
}

impl Hdf5RangeWorkspaceV1 {
    const MAX_PLAN_CHECKPOINT_BYTES: u64 = 16 * 1024 * 1024;
    const MAX_TRANSFER_CHECKPOINT_BYTES: u64 =
        NISAR_SUBSET_MAX_TRANSFER_CANONICAL_JSON_BYTES as u64;
    const MAX_METRICS_CHECKPOINT_BYTES: u64 = 64 * 1024;
    const MAX_PART_IDENTITY_CHECKPOINT_BYTES: u64 = 64 * 1024;
    const MAX_SCIENCE_CHECKPOINT_V2_BYTES: u64 = 16 * 1024 * 1024;
    const MAX_PLANNING_PREFIX_CHECKPOINT_V2_BYTES: u64 = 16 * 1024 * 1024;

    pub fn new(root: PathBuf) -> Result<Self, Hdf5RangeTransferError> {
        let root = Arc::new(
            PinnedDirectory::create(&root).map_err(|_| Hdf5RangeTransferError::WorkspaceInvalid)?,
        );
        Ok(Self { root })
    }

    /// Read-pin a historical workspace without creating any source entry.
    pub(crate) fn open_existing(root: PathBuf) -> Result<Self, Hdf5RangeTransferError> {
        let root = Arc::new(
            PinnedDirectory::open_existing(&root)
                .map_err(|_| Hdf5RangeTransferError::WorkspaceInvalid)?,
        );
        Ok(Self { root })
    }

    fn verify_held_lock(&self, lock: &Hdf5WorkspaceLockV1) -> Result<(), Hdf5RangeTransferError> {
        self.root
            .verify()
            .map_err(|_| Hdf5RangeTransferError::WorkspaceInvalid)?;
        if lock.file.is_none() || lock.path != self.lock_path() {
            return Err(Hdf5RangeTransferError::WorkspaceInvalid);
        }
        Ok(())
    }

    /// Volume facts of the workspace root for identity-verified rebinding.
    #[cfg(feature = "nisar-subset-production")]
    pub(crate) fn root_volume_v2(
        &self,
    ) -> std::io::Result<crate::nisar_workspace_rebinding_v2::NisarWorkspaceVolumeV2> {
        let (file_system, local) = self.root.volume_facts()?;
        Ok(crate::nisar_workspace_rebinding_v2::NisarWorkspaceVolumeV2 { file_system, local })
    }

    /// Non-authoritative rebinding evidence (design 2026-10-17 §4).
    #[cfg(feature = "nisar-subset-production")]
    pub(crate) fn load_rebinding_evidence_v2(
        &self,
    ) -> Result<Option<String>, Hdf5RangeTransferError> {
        self.root
            .verify()
            .map_err(|_| Hdf5RangeTransferError::WorkspaceInvalid)?;
        read_guarded_checkpoint(&self.fixed_child("workspace-rebinding-v2.json"), 64 * 1024)
    }

    #[cfg(feature = "nisar-subset-production")]
    pub(crate) fn persist_rebinding_evidence_v2(
        &self,
        canonical: &str,
    ) -> Result<(), Hdf5RangeTransferError> {
        let lock = self.acquire_lock()?;
        self.verify_held_lock(&lock)?;
        if canonical.is_empty() || canonical.len() > 64 * 1024 {
            return Err(Hdf5RangeTransferError::Persistence);
        }
        persist_atomic_text(
            self,
            &self.fixed_child("workspace-rebinding-v2.json.next"),
            &self.fixed_child("workspace-rebinding-v2.json"),
            canonical,
        )
    }

    pub(crate) fn load_archived_import_marker_v2(
        &self,
    ) -> Result<Option<String>, Hdf5RangeTransferError> {
        self.root
            .verify()
            .map_err(|_| Hdf5RangeTransferError::WorkspaceInvalid)?;
        read_guarded_checkpoint(&self.fixed_child("archived-import-v2.json"), 64 * 1024)
    }

    pub(crate) fn persist_archived_import_marker_v2(
        &self,
        canonical: &str,
        lock: &Hdf5WorkspaceLockV1,
    ) -> Result<(), Hdf5RangeTransferError> {
        self.verify_held_lock(lock)?;
        if canonical.is_empty() || canonical.len() > 64 * 1024 {
            return Err(Hdf5RangeTransferError::Persistence);
        }
        persist_atomic_text(
            self,
            &self.fixed_child("archived-import-v2.json.next"),
            &self.fixed_child("archived-import-v2.json"),
            canonical,
        )
    }

    /// Only the import owner can create/resume this part; never truncate an existing file.
    pub(crate) fn open_archived_import_part_v2(
        &self,
        source_size: u64,
        lock: &Hdf5WorkspaceLockV1,
    ) -> Result<File, Hdf5RangeTransferError> {
        self.verify_held_lock(lock)?;
        if self.load_archived_import_marker_v2()?.is_none()
            || self.load_planning_checkpoint_v2()?.is_some()
            || self.load_science_checkpoint_v2()?.is_some()
        {
            return Err(Hdf5RangeTransferError::RecoveryInvalid);
        }
        let (file, created) = open_part_file(&self.part_path())
            .map_err(|_| Hdf5RangeTransferError::RecoveryInvalid)?;
        part_file_identity(&file).map_err(|_| Hdf5RangeTransferError::RecoveryInvalid)?;
        let length = file
            .metadata()
            .map_err(|_| Hdf5RangeTransferError::RecoveryInvalid)?
            .len();
        // A crash after create_new but before set_len leaves an empty, owned file.
        // Completing that initialization cannot discard bytes; never resize nonempty data.
        if created || length == 0 {
            mark_part_file_sparse(&file)?;
            file.set_len(source_size)
                .and_then(|_| file.sync_all())
                .map_err(|_| Hdf5RangeTransferError::Persistence)?;
        } else if length != source_size {
            return Err(Hdf5RangeTransferError::RecoveryInvalid);
        }
        Ok(file)
    }

    /// Remove the task-scoped temporary Range workspace after the final HDF5
    /// artifact and manifest have been validated. A retained clone means a
    /// transfer/recovery owner is still alive, so cleanup fails closed rather
    /// than deleting a workspace that may still be in use.
    pub fn cleanup_after_success(self) -> Result<(), Hdf5RangeTransferError> {
        let root =
            Arc::try_unwrap(self.root).map_err(|_| Hdf5RangeTransferError::WorkspaceInvalid)?;
        root.remove_after_verify()
            .map_err(|_| Hdf5RangeTransferError::WorkspaceInvalid)
    }

    pub fn part_path(&self) -> PathBuf {
        self.fixed_child("ranges.part")
    }

    #[cfg(test)]
    pub(crate) fn writer_snapshot_path_v2_for_test(&self) -> PathBuf {
        self.fixed_child("science-part-writer-v2.snapshot.h5")
    }

    pub fn manifest_path(&self) -> PathBuf {
        self.fixed_child("transfer-manifest-v1.json")
    }

    pub fn plan_path(&self) -> PathBuf {
        self.fixed_child("range-plan-v1.json")
    }

    pub fn metrics_path(&self) -> PathBuf {
        self.fixed_child("range-metrics-v1.json")
    }

    pub fn part_identity_path(&self) -> PathBuf {
        self.fixed_child("range-part-identity-v1.json")
    }

    pub(crate) fn planning_checkpoint_path_v2(&self) -> PathBuf {
        self.fixed_child("planning-workspace-v2.json")
    }

    pub(crate) fn planning_prefix_checkpoint_path_v2(&self) -> PathBuf {
        self.fixed_child("planning-prefix-v2.json")
    }

    #[cfg(test)]
    pub(crate) fn planning_prefix_checkpoint_path_v2_for_test(&self) -> PathBuf {
        self.planning_prefix_checkpoint_path_v2()
    }

    #[cfg(test)]
    pub(crate) fn planning_prefix_next_checkpoint_path_v2_for_test(&self) -> PathBuf {
        self.next_planning_prefix_checkpoint_path_v2()
    }

    fn next_planning_checkpoint_path_v2(&self) -> PathBuf {
        self.fixed_child("planning-workspace-v2.json.next")
    }

    fn next_planning_prefix_checkpoint_path_v2(&self) -> PathBuf {
        self.fixed_child("planning-prefix-v2.json.next")
    }

    pub(crate) fn science_checkpoint_path_v2(&self) -> PathBuf {
        self.fixed_child("science-checkpoint-v2.json")
    }

    fn next_science_checkpoint_path_v2(&self) -> PathBuf {
        self.fixed_child("science-checkpoint-v2.json.next")
    }

    pub(crate) fn load_science_checkpoint_v2(
        &self,
    ) -> Result<Option<String>, Hdf5RangeTransferError> {
        self.root
            .verify()
            .map_err(|_| Hdf5RangeTransferError::WorkspaceInvalid)?;
        read_guarded_checkpoint(
            &self.science_checkpoint_path_v2(),
            Self::MAX_SCIENCE_CHECKPOINT_V2_BYTES,
        )
    }

    pub(crate) fn persist_science_checkpoint_v2(
        &self,
        canonical: &str,
    ) -> Result<(), Hdf5RangeTransferError> {
        if canonical.is_empty() || canonical.len() as u64 > Self::MAX_SCIENCE_CHECKPOINT_V2_BYTES {
            return Err(Hdf5RangeTransferError::Persistence);
        }
        persist_atomic_text(
            self,
            &self.next_science_checkpoint_path_v2(),
            &self.science_checkpoint_path_v2(),
            canonical,
        )
    }

    pub(crate) fn open_science_part_v2(
        &self,
        source_size: u64,
    ) -> Result<(File, String), Hdf5RangeTransferError> {
        self.root
            .verify()
            .map_err(|_| Hdf5RangeTransferError::WorkspaceInvalid)?;
        let file = open_existing_part_file(&self.part_path())
            .map_err(|_| Hdf5RangeTransferError::RecoveryInvalid)?;
        let identity =
            part_file_identity(&file).map_err(|_| Hdf5RangeTransferError::RecoveryInvalid)?;
        if file
            .metadata()
            .map_err(|_| Hdf5RangeTransferError::RecoveryInvalid)?
            .len()
            != source_size
        {
            return Err(Hdf5RangeTransferError::RecoveryInvalid);
        }
        let identity_sha256 = sha256(
            &serde_json::to_vec(&identity).map_err(|_| Hdf5RangeTransferError::RecoveryInvalid)?,
        );
        Ok((file, identity_sha256))
    }

    /// Reopens the current completed Task 2 part without following links and
    /// returns both its stable file identity and a non-resettable mutation
    /// marker. The Writer compares this marker around every source read.
    pub(crate) fn reopen_science_part_for_writer_v2(
        &self,
        source_size: u64,
    ) -> Result<(File, String, String), Hdf5RangeTransferError> {
        self.root
            .verify()
            .map_err(|_| Hdf5RangeTransferError::WorkspaceInvalid)?;
        let file = open_existing_part_for_writer(&self.part_path())
            .map_err(|_| Hdf5RangeTransferError::RecoveryInvalid)?;
        let identity =
            part_file_identity(&file).map_err(|_| Hdf5RangeTransferError::RecoveryInvalid)?;
        if file
            .metadata()
            .map_err(|_| Hdf5RangeTransferError::RecoveryInvalid)?
            .len()
            != source_size
        {
            return Err(Hdf5RangeTransferError::RecoveryInvalid);
        }
        let identity_sha256 = sha256(
            &serde_json::to_vec(&identity).map_err(|_| Hdf5RangeTransferError::RecoveryInvalid)?,
        );
        let mutation_marker_sha256 = writer_part_mutation_marker(&file)?;
        Ok((file, identity_sha256, mutation_marker_sha256))
    }

    /// Pins the source pathname and its HDF5 view. Windows uses deny-write and
    /// delete sharing; Linux/Android read the pinned source through its
    /// descriptor and detect a write by its change time in the source marker.
    pub(crate) fn pin_science_part_for_writer_v2(
        &self,
        source_size: u64,
    ) -> Result<ImmutableSciencePartForWriterV2, Hdf5RangeTransferError> {
        let (source, source_identity_sha256, mutation_before) =
            self.reopen_science_part_for_writer_v2(source_size)?;
        let snapshot_path = self.fixed_child("science-part-writer-v2.snapshot.h5");
        let (view, view_path) =
            immutable_writer_part_view(&source, &self.part_path(), &snapshot_path)?;
        if view
            .metadata()
            .map_err(|_| Hdf5RangeTransferError::RecoveryInvalid)?
            .len()
            != source_size
        {
            return Err(Hdf5RangeTransferError::RecoveryInvalid);
        }
        let mutation_after = writer_part_mutation_marker(&source)?;
        if mutation_after != mutation_before {
            return Err(Hdf5RangeTransferError::RecoveryInvalid);
        }
        Ok(ImmutableSciencePartForWriterV2 {
            source,
            view,
            view_path,
            source_identity_sha256,
            source_mutation_marker_sha256: mutation_before,
        })
    }

    pub(crate) fn science_part_mutation_marker_for_writer_v2(
        &self,
        file: &File,
    ) -> Result<String, Hdf5RangeTransferError> {
        writer_part_mutation_marker(file)
    }

    pub(crate) fn planning_workspace_identity_v2(
        &self,
    ) -> Result<(String, String), Hdf5RangeTransferError> {
        self.root
            .verify()
            .map_err(|_| Hdf5RangeTransferError::WorkspaceInvalid)?;
        Ok((
            self.root.path().to_string_lossy().into_owned(),
            self.root.identity_marker(),
        ))
    }

    pub(crate) fn reset_planning_part_v2(
        &self,
        source_size: u64,
    ) -> Result<File, Hdf5RangeTransferError> {
        self.root
            .verify()
            .map_err(|_| Hdf5RangeTransferError::WorkspaceInvalid)?;
        if source_size == 0 {
            return Err(Hdf5RangeTransferError::RangeInvalid);
        }
        let (file, _) =
            open_part_file(&self.part_path()).map_err(|_| Hdf5RangeTransferError::Persistence)?;
        part_file_identity(&file)?;
        file.set_len(0)
            .map_err(|_| Hdf5RangeTransferError::Persistence)?;
        mark_part_file_sparse(&file)?;
        file.set_len(source_size)
            .and_then(|_| file.sync_all())
            .map_err(|_| Hdf5RangeTransferError::Persistence)?;
        Ok(file)
    }

    pub(crate) fn reopen_planning_prefix_part_v2(
        &self,
        source_size: u64,
    ) -> Result<File, Hdf5RangeTransferError> {
        self.root
            .verify()
            .map_err(|_| Hdf5RangeTransferError::WorkspaceInvalid)?;
        if source_size == 0 {
            return Err(Hdf5RangeTransferError::RangeInvalid);
        }
        let file = open_existing_part_file(&self.part_path())
            .map_err(|_| Hdf5RangeTransferError::RecoveryInvalid)?;
        part_file_identity(&file).map_err(|_| Hdf5RangeTransferError::RecoveryInvalid)?;
        if file
            .metadata()
            .map_err(|_| Hdf5RangeTransferError::RecoveryInvalid)?
            .len()
            != source_size
        {
            return Err(Hdf5RangeTransferError::RecoveryInvalid);
        }
        Ok(file)
    }

    pub(crate) fn load_planning_prefix_checkpoint_v2(
        &self,
    ) -> Result<Option<String>, Hdf5RangeTransferError> {
        self.root
            .verify()
            .map_err(|_| Hdf5RangeTransferError::WorkspaceInvalid)?;
        read_guarded_checkpoint(
            &self.planning_prefix_checkpoint_path_v2(),
            Self::MAX_PLANNING_PREFIX_CHECKPOINT_V2_BYTES,
        )
    }

    pub(crate) fn persist_planning_prefix_checkpoint_v2(
        &self,
        canonical: &str,
    ) -> Result<(), Hdf5RangeTransferError> {
        if canonical.is_empty()
            || canonical.len() as u64 > Self::MAX_PLANNING_PREFIX_CHECKPOINT_V2_BYTES
        {
            return Err(Hdf5RangeTransferError::Persistence);
        }
        let lock = self.acquire_lock()?;
        self.persist_planning_prefix_checkpoint_locked_v2(canonical, &lock)
    }

    pub(crate) fn persist_planning_prefix_checkpoint_locked_v2(
        &self,
        canonical: &str,
        lock: &Hdf5WorkspaceLockV1,
    ) -> Result<(), Hdf5RangeTransferError> {
        self.verify_held_lock(lock)?;
        if canonical.is_empty()
            || canonical.len() as u64 > Self::MAX_PLANNING_PREFIX_CHECKPOINT_V2_BYTES
        {
            return Err(Hdf5RangeTransferError::Persistence);
        }
        persist_atomic_text(
            self,
            &self.next_planning_prefix_checkpoint_path_v2(),
            &self.planning_prefix_checkpoint_path_v2(),
            canonical,
        )
    }

    #[cfg(feature = "nisar-subset-production")]
    /// Downloader enhancement D1c: `stop` is asked before every 64 KiB read of
    /// the planning material (195 MB for one L4 item), so a pause during a
    /// planning revalidation takes effect at once and returns `Cancelled`.
    pub(crate) fn inspect_planning_part_evidence_v2(
        &self,
        source_size: u64,
        material_ranges: &[(u64, u64)],
        stop: &dyn Fn() -> bool,
    ) -> Result<Option<NisarPlanningPartEvidenceV2>, Hdf5RangeTransferError> {
        self.root
            .verify()
            .map_err(|_| Hdf5RangeTransferError::WorkspaceInvalid)?;
        if source_size == 0
            || material_ranges
                .iter()
                .any(|range| range.0 >= range.1 || range.1 > source_size)
            || material_ranges.windows(2).any(|pair| pair[0].1 > pair[1].0)
        {
            return Err(Hdf5RangeTransferError::RecoveryInvalid);
        }
        let material_bytes = material_ranges
            .iter()
            .try_fold(0_u64, |total, range| total.checked_add(range.1 - range.0));
        if material_bytes.is_none_or(|bytes| {
            bytes > nisar_live_budget_response_limit(source_size).unwrap_or_default()
        }) {
            return Err(Hdf5RangeTransferError::RecoveryInvalid);
        }
        let mut file = match open_checkpoint_nofollow(&self.part_path()) {
            Ok(file) => file,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(_) => return Err(Hdf5RangeTransferError::RecoveryInvalid),
        };
        let identity =
            part_file_identity(&file).map_err(|_| Hdf5RangeTransferError::RecoveryInvalid)?;
        if file
            .metadata()
            .map_err(|_| Hdf5RangeTransferError::RecoveryInvalid)?
            .len()
            != source_size
        {
            return Err(Hdf5RangeTransferError::RecoveryInvalid);
        }
        let identity_sha256 = sha256(
            &serde_json::to_vec(&identity).map_err(|_| Hdf5RangeTransferError::RecoveryInvalid)?,
        );
        let mut material_digest = Sha256::new();
        material_digest.update(b"NisarPlanningSparsePartMaterialV2");
        let mut buffer = [0_u8; 64 * 1024];
        for &(start, end_exclusive) in material_ranges {
            material_digest.update(start.to_be_bytes());
            material_digest.update(end_exclusive.to_be_bytes());
            file.seek(SeekFrom::Start(start))
                .map_err(|_| Hdf5RangeTransferError::RecoveryInvalid)?;
            let mut remaining = end_exclusive - start;
            while remaining > 0 {
                #[cfg(test)]
                long_step_hook_for_test::step();
                if stop() {
                    return Err(Hdf5RangeTransferError::Cancelled);
                }
                let count = usize::try_from(remaining.min(buffer.len() as u64))
                    .map_err(|_| Hdf5RangeTransferError::RecoveryInvalid)?;
                file.read_exact(&mut buffer[..count])
                    .map_err(|_| Hdf5RangeTransferError::RecoveryInvalid)?;
                material_digest.update(&buffer[..count]);
                remaining -= count as u64;
            }
        }
        Ok(Some(NisarPlanningPartEvidenceV2 {
            identity_sha256,
            material_sha256: format!("{:x}", material_digest.finalize()),
        }))
    }

    pub(crate) fn load_planning_checkpoint_v2(
        &self,
    ) -> Result<Option<String>, Hdf5RangeTransferError> {
        self.root
            .verify()
            .map_err(|_| Hdf5RangeTransferError::WorkspaceInvalid)?;
        read_guarded_checkpoint(
            &self.planning_checkpoint_path_v2(),
            Self::MAX_PLAN_CHECKPOINT_BYTES,
        )
    }

    pub(crate) fn persist_planning_checkpoint_v2(
        &self,
        canonical: &str,
    ) -> Result<(), Hdf5RangeTransferError> {
        if canonical.is_empty() || canonical.len() as u64 > Self::MAX_PLAN_CHECKPOINT_BYTES {
            return Err(Hdf5RangeTransferError::Persistence);
        }
        let lock = self.acquire_lock()?;
        self.persist_planning_checkpoint_locked_v2(canonical, &lock)
    }

    pub(crate) fn persist_planning_checkpoint_locked_v2(
        &self,
        canonical: &str,
        lock: &Hdf5WorkspaceLockV1,
    ) -> Result<(), Hdf5RangeTransferError> {
        self.verify_held_lock(lock)?;
        if canonical.is_empty() || canonical.len() as u64 > Self::MAX_PLAN_CHECKPOINT_BYTES {
            return Err(Hdf5RangeTransferError::Persistence);
        }
        persist_atomic_text(
            self,
            &self.next_planning_checkpoint_path_v2(),
            &self.planning_checkpoint_path_v2(),
            canonical,
        )
    }

    fn next_manifest_path(&self) -> PathBuf {
        self.fixed_child("transfer-manifest-v1.json.next")
    }

    fn next_plan_path(&self) -> PathBuf {
        self.fixed_child("range-plan-v1.json.next")
    }

    fn next_metrics_path(&self) -> PathBuf {
        self.fixed_child("range-metrics-v1.json.next")
    }

    fn next_part_identity_path(&self) -> PathBuf {
        self.fixed_child("range-part-identity-v1.json.next")
    }

    fn lock_path(&self) -> PathBuf {
        self.fixed_child("range-workspace-v1.lock")
    }

    fn fixed_child(&self, name: &str) -> PathBuf {
        self.root
            .stable_component(name)
            .expect("workspace construction validated stable fixed child paths")
    }

    pub(crate) fn acquire_lock(&self) -> Result<Hdf5WorkspaceLockV1, Hdf5RangeTransferError> {
        self.root
            .verify()
            .map_err(|_| Hdf5RangeTransferError::WorkspaceInvalid)?;
        acquire_hdf5_workspace_lock(&self.lock_path())
    }

    fn bind_plan(&self, plan: &Hdf5RangePlanV1) -> Result<(), Hdf5RangeTransferError> {
        self.root
            .verify()
            .map_err(|_| Hdf5RangeTransferError::WorkspaceInvalid)?;
        if compute_plan_sha256(plan)? != plan.plan_sha256 {
            return Err(Hdf5RangeTransferError::RangeInvalid);
        }
        let path = self.plan_path();
        if let Some(text) = read_guarded_checkpoint(&path, Self::MAX_PLAN_CHECKPOINT_BYTES)? {
            let stored = serde_json::from_str::<StoredHdf5RangePlanV1>(&text)
                .map_err(|_| Hdf5RangeTransferError::RecoveryInvalid)?;
            let stored = stored.into_plan();
            if compute_plan_sha256(&stored)? != stored.plan_sha256 || stored != *plan {
                return Err(Hdf5RangeTransferError::RecoveryInvalid);
            }
            return Ok(());
        }
        let canonical =
            serde_json::to_string(plan).map_err(|_| Hdf5RangeTransferError::Persistence)?;
        persist_atomic_text(self, &self.next_plan_path(), &self.plan_path(), &canonical)
    }

    pub fn load_manifest(
        &self,
    ) -> Result<Option<NisarSubsetTransferManifestV1>, Hdf5RangeTransferError> {
        self.root
            .verify()
            .map_err(|_| Hdf5RangeTransferError::WorkspaceInvalid)?;
        let path = self.manifest_path();
        let Some(text) = read_guarded_checkpoint(&path, Self::MAX_TRANSFER_CHECKPOINT_BYTES)?
        else {
            return Ok(None);
        };
        let manifest = serde_json::from_str::<NisarSubsetTransferManifestV1>(&text)
            .map_err(|_| Hdf5RangeTransferError::RecoveryInvalid)?;
        manifest
            .validate()
            .map_err(|_| Hdf5RangeTransferError::RecoveryInvalid)?;
        Ok(Some(manifest))
    }

    fn load_metrics(
        &self,
        plan: &Hdf5RangePlanV1,
    ) -> Result<Hdf5RangeTransferMetricsV1, Hdf5RangeTransferError> {
        self.root
            .verify()
            .map_err(|_| Hdf5RangeTransferError::WorkspaceInvalid)?;
        let path = self.metrics_path();
        let Some(text) = read_guarded_checkpoint(&path, Self::MAX_METRICS_CHECKPOINT_BYTES)? else {
            return Ok(Hdf5RangeTransferMetricsV1::default());
        };
        let checkpoint = serde_json::from_str::<Hdf5RangeMetricsCheckpointV1>(&text)
            .map_err(|_| Hdf5RangeTransferError::RecoveryInvalid)?;
        if checkpoint.metrics_contract != "NisarOfflineHdf5RangeMetricsV1"
            || checkpoint.request_manifest_sha256 != plan.request_manifest_sha256
            || checkpoint.plan_sha256 != plan.plan_sha256
            || checkpoint.metrics.accepted_unique_bytes > plan.expected_remote_bytes
            || checkpoint.metrics.accepted_unique_bytes > checkpoint.metrics.response_body_bytes
            || checkpoint.metrics.retry_duplicate_bytes
                != checkpoint
                    .metrics
                    .response_body_bytes
                    .saturating_sub(checkpoint.metrics.accepted_unique_bytes)
        {
            return Err(Hdf5RangeTransferError::RecoveryInvalid);
        }
        Ok(checkpoint.metrics)
    }

    #[cfg(all(test, feature = "nisar-subset-production"))]
    #[allow(dead_code)]
    pub(crate) fn recover_completed_result_read_only_for_test(
        &self,
        plan: &Hdf5RangePlanV1,
    ) -> Result<Hdf5RangeTransferResultV1, Hdf5RangeTransferError> {
        // Diagnostics over preserved evidence must not rewrite checkpoints.
        // Reuse the production validators while deliberately omitting the
        // execution path's metrics reconciliation persistence.
        self.bind_plan(plan)?;
        let mut metrics = self.load_metrics(plan)?;
        let manifest = self
            .load_manifest()?
            .ok_or(Hdf5RangeTransferError::RecoveryInvalid)?;
        let (mut part, identity) = open_part(&self.part_path(), plan.source_length)?;
        self.bind_part_identity(plan, &identity)?;
        validate_manifest_plan(&manifest, plan, &manifest.source_validator.value)?;
        verify_completed_ranges(&manifest, &mut part)?;
        reconcile_metrics_with_manifest(&mut metrics, &manifest)?;
        if manifest.state != NisarSubsetTransferState::RangesComplete {
            return Err(Hdf5RangeTransferError::RecoveryInvalid);
        }
        Ok(Hdf5RangeTransferResultV1 {
            manifest,
            metrics,
            plan_sha256: plan.plan_sha256.clone(),
        })
    }

    fn persist_metrics(
        &self,
        plan: &Hdf5RangePlanV1,
        metrics: Hdf5RangeTransferMetricsV1,
    ) -> Result<(), Hdf5RangeTransferError> {
        self.root
            .verify()
            .map_err(|_| Hdf5RangeTransferError::WorkspaceInvalid)?;
        let checkpoint = Hdf5RangeMetricsCheckpointV1 {
            metrics_contract: "NisarOfflineHdf5RangeMetricsV1".to_owned(),
            request_manifest_sha256: plan.request_manifest_sha256.clone(),
            plan_sha256: plan.plan_sha256.clone(),
            metrics,
        };
        let canonical =
            serde_json::to_string(&checkpoint).map_err(|_| Hdf5RangeTransferError::Persistence)?;
        persist_atomic_text(
            self,
            &self.next_metrics_path(),
            &self.metrics_path(),
            &canonical,
        )
    }

    fn bind_part_identity(
        &self,
        plan: &Hdf5RangePlanV1,
        identity: &PartFileIdentityV1,
    ) -> Result<(), Hdf5RangeTransferError> {
        self.root
            .verify()
            .map_err(|_| Hdf5RangeTransferError::WorkspaceInvalid)?;
        let path = self.part_identity_path();
        if let Some(text) =
            read_guarded_checkpoint(&path, Self::MAX_PART_IDENTITY_CHECKPOINT_BYTES)?
        {
            let stored = serde_json::from_str::<PartFileIdentityCheckpointV1>(&text)
                .map_err(|_| Hdf5RangeTransferError::RecoveryInvalid)?;
            if stored.identity_contract != "NisarOfflineHdf5PartIdentityV1"
                || stored.request_manifest_sha256 != plan.request_manifest_sha256
                || stored.plan_sha256 != plan.plan_sha256
                || stored.identity != *identity
            {
                return Err(Hdf5RangeTransferError::RecoveryInvalid);
            }
            return Ok(());
        }
        let checkpoint = PartFileIdentityCheckpointV1 {
            identity_contract: "NisarOfflineHdf5PartIdentityV1".to_owned(),
            request_manifest_sha256: plan.request_manifest_sha256.clone(),
            plan_sha256: plan.plan_sha256.clone(),
            identity: identity.clone(),
        };
        let canonical =
            serde_json::to_string(&checkpoint).map_err(|_| Hdf5RangeTransferError::Persistence)?;
        persist_atomic_text(
            self,
            &self.next_part_identity_path(),
            &self.part_identity_path(),
            &canonical,
        )
    }

    pub(crate) fn acquire_verified_chunk_blocks(
        &self,
        request: &NisarSubsetRequestManifestV1,
        plan: &Hdf5RangePlanV1,
        result: &Hdf5RangeTransferResultV1,
        cancelled: &(dyn Fn() -> bool + Sync),
    ) -> Result<(Hdf5WorkspaceLockV1, Vec<VerifiedHdf5ChunkBlockV1>), Hdf5RangeTransferError> {
        request
            .validate()
            .map_err(|_| Hdf5RangeTransferError::RecoveryInvalid)?;
        result
            .manifest
            .validate_against_request(request)
            .map_err(|_| Hdf5RangeTransferError::RecoveryInvalid)?;
        if result.manifest.state != NisarSubsetTransferState::RangesComplete
            || result.manifest.ranges.iter().any(|range| {
                range.state != NisarSubsetRangeState::Complete || range.sha256.is_none()
            })
            || result.plan_sha256 != plan.plan_sha256
            || result.manifest.received_bytes != plan.expected_remote_bytes
        {
            return Err(Hdf5RangeTransferError::RecoveryInvalid);
        }
        let workspace_lock = self.acquire_lock()?;
        if read_guarded_checkpoint(&self.plan_path(), Self::MAX_PLAN_CHECKPOINT_BYTES)?.is_none()
            || read_guarded_checkpoint(
                &self.part_identity_path(),
                Self::MAX_PART_IDENTITY_CHECKPOINT_BYTES,
            )?
            .is_none()
        {
            return Err(Hdf5RangeTransferError::RecoveryInvalid);
        }
        self.bind_plan(plan)?;
        if self.load_manifest()?.as_ref() != Some(&result.manifest) {
            return Err(Hdf5RangeTransferError::RecoveryInvalid);
        }
        let (mut part, identity) = open_part(&self.part_path(), plan.source_length)?;
        self.bind_part_identity(plan, &identity)?;
        validate_manifest_plan(
            &result.manifest,
            plan,
            &result.manifest.source_validator.value,
        )?;

        // Phase 1: verify every planned acquisition interval cryptographically
        // before any chunk byte may be trusted. A planned Range becomes
        // `VerifiedCoverage` only after the part bytes recompute exactly the
        // digest persisted in the completed manifest Range.
        let mut verified = Vec::with_capacity(plan.ranges.len());
        for (planned, persisted) in plan.ranges.iter().zip(&result.manifest.ranges) {
            if cancelled() {
                return Err(Hdf5RangeTransferError::Cancelled);
            }
            part.seek(SeekFrom::Start(planned.start))
                .map_err(|_| Hdf5RangeTransferError::RecoveryInvalid)?;
            let mut digest = Sha256::new();
            let mut cursor = planned.start;
            let mut buffer = [0_u8; 64 * 1024];
            while cursor < planned.end_exclusive {
                if cancelled() {
                    return Err(Hdf5RangeTransferError::Cancelled);
                }
                let count =
                    usize::try_from((planned.end_exclusive - cursor).min(buffer.len() as u64))
                        .map_err(|_| Hdf5RangeTransferError::RangeBytesExceeded)?;
                part.read_exact(&mut buffer[..count])
                    .map_err(|_| Hdf5RangeTransferError::RecoveryInvalid)?;
                digest.update(&buffer[..count]);
                cursor = cursor
                    .checked_add(count as u64)
                    .ok_or(Hdf5RangeTransferError::RangeBytesExceeded)?;
            }
            if persisted.sha256.as_deref() != Some(format!("{:x}", digest.finalize()).as_str()) {
                return Err(Hdf5RangeTransferError::RecoveryInvalid);
            }
            verified.push((planned.start, planned.end_exclusive));
        }
        let coverage = normalize_verified_ranges(&verified, plan.source_length)?;

        // Phase 2: materialize a chunk only when the COMPLETE physical chunk
        // interval is covered by the union of verified acquisition ranges.
        // Prefix-resident chunks are covered by the prefix Range; straddling
        // chunks by the union of prefix and science coverage; a sparse hole
        // between verified ranges fails closed below. Part-file existence is
        // never proof of valid data: the trust chain is HTTP/local Range ->
        // planned interval -> completed manifest interval -> digest verified ->
        // verified coverage -> chunk interval completely covered -> part read.
        let mut shared = BTreeMap::<(u64, u64), Arc<[u8]>>::new();
        let mut blocks = Vec::with_capacity(plan.chunks.len());
        for chunk in &plan.chunks {
            let allocation = match &chunk.allocation {
                Hdf5ChunkAllocationV1::Allocated {
                    start,
                    end_exclusive,
                    ..
                } => {
                    if !interval_is_fully_covered((*start, *end_exclusive), &coverage) {
                        return Err(Hdf5RangeTransferError::RecoveryInvalid);
                    }
                    let key = (*start, *end_exclusive);
                    let bytes = match shared.entry(key) {
                        std::collections::btree_map::Entry::Vacant(entry) => {
                            let length = usize::try_from(end_exclusive - start)
                                .map_err(|_| Hdf5RangeTransferError::RangeBytesExceeded)?;
                            let mut bytes = vec![0_u8; length];
                            part.seek(SeekFrom::Start(*start))
                                .map_err(|_| Hdf5RangeTransferError::RecoveryInvalid)?;
                            part.read_exact(&mut bytes)
                                .map_err(|_| Hdf5RangeTransferError::RecoveryInvalid)?;
                            Arc::clone(entry.insert(Arc::from(bytes)))
                        }
                        std::collections::btree_map::Entry::Occupied(entry) => {
                            Arc::clone(entry.get())
                        }
                    };
                    VerifiedHdf5ChunkAllocationV1::Allocated { bytes }
                }
                Hdf5ChunkAllocationV1::Fill { element_fill_hex } => {
                    VerifiedHdf5ChunkAllocationV1::Fill {
                        element_fill_hex: element_fill_hex.clone(),
                    }
                }
            };
            blocks.push(VerifiedHdf5ChunkBlockV1 {
                dataset_path: chunk.dataset_path.clone(),
                component_id: chunk.component_id,
                logical_chunk: chunk.logical_chunk,
                dataset_shape: chunk.dataset_shape,
                chunk_shape: chunk.chunk_shape,
                datatype: chunk.datatype,
                datatype_sha256: chunk.datatype_sha256.clone(),
                fill_value_definition: chunk.fill_value_definition,
                fill_value_hex: chunk.fill_value_hex.clone(),
                filter_pipeline: chunk.filter_pipeline,
                part_path: self.part_path(),
                allocation,
            });
        }
        Ok((workspace_lock, blocks))
    }
}

pub(crate) fn acquire_hdf5_workspace_lock(
    path: &Path,
) -> Result<Hdf5WorkspaceLockV1, Hdf5RangeTransferError> {
    let mut file = open_workspace_lock(path)?;
    match file.try_lock() {
        Ok(()) => {}
        Err(TryLockError::WouldBlock) => {
            return Err(Hdf5RangeTransferError::WorkspaceBusy);
        }
        Err(TryLockError::Error(_)) => {
            return Err(Hdf5RangeTransferError::WorkspaceInvalid);
        }
    }
    file.set_len(0)
        .and_then(|_| file.write_all(workspace_lock_owner().as_bytes()))
        .and_then(|_| file.sync_all())
        .map_err(|_| Hdf5RangeTransferError::WorkspaceInvalid)?;
    Ok(Hdf5WorkspaceLockV1 {
        file: Some(file),
        path: path.to_path_buf(),
    })
}

/// Takes a shared OS lock on the lock file at `path` without writing to it;
/// `WorkspaceBusy` while another handle holds it exclusively. Downloader
/// enhancement D1: NISAR V2 writers hold the directory lock of earlier builds
/// shared, so an earlier build (which takes it exclusively while it writes)
/// never writes beside them, while they do not serialize on it themselves.
#[cfg(feature = "nisar-subset-production")]
pub(crate) fn try_acquire_hdf5_workspace_lock_shared(
    path: &Path,
) -> Result<Hdf5WorkspaceLockV1, Hdf5RangeTransferError> {
    let file = open_workspace_lock(path)?;
    match file.try_lock_shared() {
        Ok(()) => {}
        Err(TryLockError::WouldBlock) => {
            return Err(Hdf5RangeTransferError::WorkspaceBusy);
        }
        Err(TryLockError::Error(_)) => {
            return Err(Hdf5RangeTransferError::WorkspaceInvalid);
        }
    }
    Ok(Hdf5WorkspaceLockV1 {
        file: Some(file),
        path: path.to_path_buf(),
    })
}

/// Test-only hook run before every step of the long loops of a planning
/// revalidation (downloader enhancement D1c): a test slows each step down to
/// emulate a real source (195 MB of planning material, tens of thousands of
/// chunk lookups) and pauses midway.
#[cfg(test)]
pub(crate) mod long_step_hook_for_test {
    use std::cell::RefCell;

    type Hook = Box<dyn Fn()>;

    thread_local! {
        static HOOK: RefCell<Option<Hook>> = const { RefCell::new(None) };
    }

    pub(crate) struct Installed(());

    pub(crate) fn install(hook: impl Fn() + 'static) -> Installed {
        HOOK.with(|slot| *slot.borrow_mut() = Some(Box::new(hook)));
        Installed(())
    }

    impl Drop for Installed {
        fn drop(&mut self) {
            HOOK.with(|slot| *slot.borrow_mut() = None);
        }
    }

    pub(crate) fn step() {
        HOOK.with(|slot| {
            if let Some(hook) = slot.borrow().as_ref() {
                hook();
            }
        });
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
struct PartFileIdentityV1 {
    platform: String,
    device_id: u64,
    file_id: u64,
    /// Unix birth time in nanoseconds, where the filesystem reports one. A
    /// replacement can reuse the freed inode number at once (ext4 does), so
    /// device and inode alone do not tell it apart. Absent on Windows, whose
    /// file ids are not reused this way, so its recorded identities keep
    /// their exact bytes.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    birth_ns: Option<u64>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
struct PartFileIdentityCheckpointV1 {
    identity_contract: String,
    request_manifest_sha256: String,
    plan_sha256: String,
    identity: PartFileIdentityV1,
}

#[cfg(windows)]
fn open_workspace_lock(path: &Path) -> Result<File, Hdf5RangeTransferError> {
    use std::os::windows::fs::OpenOptionsExt;

    let file = OpenOptions::new()
        .create(true)
        .read(true)
        .write(true)
        .truncate(false)
        // Permit peers to open the stable lock inode and observe the OS lock,
        // but deny delete/rename sharing while any owner is active.
        .share_mode(0x0000_0001 | 0x0000_0002)
        .custom_flags(0x0020_0000)
        .open(path)
        .map_err(|_| Hdf5RangeTransferError::WorkspaceInvalid)?;
    part_file_identity(&file)?;
    Ok(file)
}

// O_NOFOLLOW is an OS ABI constant and differs on Linux-family ARM targets.
// Keep each audited desktop family explicit; unknown Unix targets fail closed
// in `open_workspace_lock`/`open_part_file` below.
#[cfg(all(
    target_os = "linux",
    any(
        target_arch = "arm",
        target_arch = "aarch64",
        target_arch = "m68k",
        target_arch = "powerpc",
        target_arch = "powerpc64"
    )
))]
const UNIX_O_NOFOLLOW: i32 = 0x0000_8000;

#[cfg(all(
    target_os = "linux",
    not(any(
        target_arch = "arm",
        target_arch = "aarch64",
        target_arch = "m68k",
        target_arch = "powerpc",
        target_arch = "powerpc64"
    ))
))]
const UNIX_O_NOFOLLOW: i32 = 0x0002_0000;

#[cfg(all(
    target_os = "android",
    any(target_arch = "arm", target_arch = "aarch64")
))]
const UNIX_O_NOFOLLOW: i32 = 0x0000_8000;

#[cfg(all(
    target_os = "android",
    any(target_arch = "x86", target_arch = "x86_64")
))]
const UNIX_O_NOFOLLOW: i32 = 0x0002_0000;

#[cfg(all(target_os = "android", target_arch = "riscv64"))]
const UNIX_O_NOFOLLOW: i32 = 0x0040_0000;

#[cfg(any(target_os = "solaris", target_os = "illumos"))]
const UNIX_O_NOFOLLOW: i32 = 0x0002_0000;

#[cfg(any(
    target_os = "macos",
    target_os = "ios",
    target_os = "freebsd",
    target_os = "dragonfly",
    target_os = "netbsd",
    target_os = "openbsd"
))]
const UNIX_O_NOFOLLOW: i32 = 0x0000_0100;

#[cfg(any(
    target_os = "linux",
    target_os = "android",
    target_os = "solaris",
    target_os = "illumos",
    target_os = "macos",
    target_os = "ios",
    target_os = "freebsd",
    target_os = "dragonfly",
    target_os = "netbsd",
    target_os = "openbsd"
))]
fn open_workspace_lock(path: &Path) -> Result<File, Hdf5RangeTransferError> {
    use std::os::unix::fs::OpenOptionsExt;

    let file = OpenOptions::new()
        .create(true)
        .read(true)
        .write(true)
        .truncate(false)
        .custom_flags(UNIX_O_NOFOLLOW)
        .open(path)
        .map_err(|_| Hdf5RangeTransferError::WorkspaceInvalid)?;
    part_file_identity(&file)?;
    Ok(file)
}

fn workspace_lock_owner() -> String {
    std::process::id().to_string()
}

/// Replaces the content of the small note at `path` with `text`, opened like a
/// workspace lock file: never through a link, a reparse point or a second
/// name of another file (downloader enhancement D1c).
#[cfg(feature = "nisar-subset-production")]
pub(crate) fn write_note_nofollow(path: &Path, text: &str) -> Result<(), Hdf5RangeTransferError> {
    let mut file = open_workspace_lock(path)?;
    file.set_len(0)
        .and_then(|_| file.write_all(text.as_bytes()))
        .map_err(|_| Hdf5RangeTransferError::Persistence)
}

#[cfg(all(
    unix,
    not(any(
        target_os = "linux",
        target_os = "android",
        target_os = "solaris",
        target_os = "illumos",
        target_os = "macos",
        target_os = "ios",
        target_os = "freebsd",
        target_os = "dragonfly",
        target_os = "netbsd",
        target_os = "openbsd"
    ))
))]
fn open_workspace_lock(_path: &Path) -> Result<File, Hdf5RangeTransferError> {
    Err(Hdf5RangeTransferError::WorkspaceInvalid)
}

#[cfg(all(not(windows), not(unix)))]
fn open_workspace_lock(_path: &Path) -> Result<File, Hdf5RangeTransferError> {
    Err(Hdf5RangeTransferError::WorkspaceInvalid)
}

pub(crate) struct Hdf5WorkspaceLockV1 {
    file: Option<File>,
    path: PathBuf,
}

impl Drop for Hdf5WorkspaceLockV1 {
    fn drop(&mut self) {
        if let Some(file) = self.file.take() {
            let _ = file.unlock();
            drop(file);
        }
    }
}

#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Hdf5RangeTransferMetricsV1 {
    pub request_count: u64,
    pub response_body_bytes: u64,
    pub accepted_unique_bytes: u64,
    pub retry_duplicate_bytes: u64,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
struct Hdf5RangeMetricsCheckpointV1 {
    metrics_contract: String,
    request_manifest_sha256: String,
    plan_sha256: String,
    metrics: Hdf5RangeTransferMetricsV1,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Hdf5RangeTransferResultV1 {
    manifest: NisarSubsetTransferManifestV1,
    metrics: Hdf5RangeTransferMetricsV1,
    plan_sha256: String,
}

impl Hdf5RangeTransferResultV1 {
    pub fn manifest(&self) -> &NisarSubsetTransferManifestV1 {
        &self.manifest
    }

    pub const fn metrics(&self) -> Hdf5RangeTransferMetricsV1 {
        self.metrics
    }

    pub fn plan_sha256(&self) -> &str {
        &self.plan_sha256
    }
}

pub struct OfflineHdf5RangeTransfer {
    http: Arc<HttpTransfer>,
    asset: ResolvedAsset,
    workspace: Hdf5RangeWorkspaceV1,
    destination_lease: Arc<DestinationLeaseController>,
    bound_request_manifest_sha256: String,
    bound_source_identity: NisarSubsetSourceIdentityV1,
}

impl OfflineHdf5RangeTransfer {
    pub fn new(
        http: HttpTransfer,
        request: &NisarSubsetRequestManifestV1,
        snapshot_item: &SnapshotItem,
        asset: ResolvedAsset,
        workspace: Hdf5RangeWorkspaceV1,
        destination_lease: Arc<DestinationLeaseController>,
    ) -> Result<Self, Hdf5RangeTransferError> {
        Self::new_shared(
            Arc::new(http),
            request,
            snapshot_item,
            asset,
            workspace,
            destination_lease,
        )
    }

    #[doc(hidden)]
    pub fn new_shared(
        http: Arc<HttpTransfer>,
        request: &NisarSubsetRequestManifestV1,
        snapshot_item: &SnapshotItem,
        asset: ResolvedAsset,
        workspace: Hdf5RangeWorkspaceV1,
        destination_lease: Arc<DestinationLeaseController>,
    ) -> Result<Self, Hdf5RangeTransferError> {
        validate_request_against_snapshot_item(request, snapshot_item)
            .map_err(|_| Hdf5RangeTransferError::WindowIdentityMismatch)?;
        if !matches!(
            http.proxy_policy(),
            ProxyPolicy::Direct | ProxyPolicy::System | ProxyPolicy::Manual(_)
        ) || asset.method() != crate::ResolvedMethod::Get
            || !range_asset_url_allowed(&asset)
        {
            return Err(Hdf5RangeTransferError::RangeUnsupported);
        }
        frozen_strong_etag(&asset)?;
        if asset.expected_size() != Some(request.source.expected_size_bytes)
            || asset.remote_validator().content_length != Some(request.source.expected_size_bytes)
            || !resolved_checksum_matches_request(&asset, request)
        {
            return Err(Hdf5RangeTransferError::SourceChanged);
        }
        Ok(Self {
            http,
            asset,
            workspace,
            destination_lease,
            bound_request_manifest_sha256: request
                .sha256()
                .map_err(|_| Hdf5RangeTransferError::RangeInvalid)?,
            bound_source_identity: request.source.clone(),
        })
    }

    #[allow(dead_code)]
    pub fn workspace(&self) -> &Hdf5RangeWorkspaceV1 {
        &self.workspace
    }

    pub fn execute(
        &self,
        request: &NisarSubsetRequestManifestV1,
        plan: &Hdf5RangePlanV1,
        timestamp: &str,
        cancelled: &(dyn Fn() -> bool + Sync),
    ) -> Result<Hdf5RangeTransferResultV1, Hdf5RangeTransferError> {
        self.execute_inner(request, plan, timestamp, cancelled, None)
    }

    #[cfg(feature = "nisar-subset-production")]
    pub(crate) fn execute_with_nisar_budget(
        &self,
        request: &NisarSubsetRequestManifestV1,
        plan: &Hdf5RangePlanV1,
        timestamp: &str,
        cancelled: &(dyn Fn() -> bool + Sync),
        budget: &NisarLiveNetworkBudgetV1,
    ) -> Result<Hdf5RangeTransferResultV1, Hdf5RangeTransferError> {
        self.execute_inner(request, plan, timestamp, cancelled, Some(budget))
    }

    fn execute_inner(
        &self,
        request: &NisarSubsetRequestManifestV1,
        plan: &Hdf5RangePlanV1,
        timestamp: &str,
        cancelled: &(dyn Fn() -> bool + Sync),
        #[cfg(feature = "nisar-subset-production")] budget: Option<&NisarLiveNetworkBudgetV1>,
        #[cfg(not(feature = "nisar-subset-production"))] _budget: Option<()>,
    ) -> Result<Hdf5RangeTransferResultV1, Hdf5RangeTransferError> {
        request
            .validate()
            .map_err(|_| Hdf5RangeTransferError::RangeInvalid)?;
        if request.sha256().ok().as_deref() != Some(self.bound_request_manifest_sha256.as_str())
            || request.source != self.bound_source_identity
            || request.sha256().ok().as_deref() != Some(plan.request_manifest_sha256())
            || plan.expected_remote_bytes() != request.expected_output.estimated_remote_bytes
            || compute_plan_sha256(plan)? != plan.plan_sha256
        {
            return Err(Hdf5RangeTransferError::WindowIdentityMismatch);
        }
        if self.asset.expected_size() != Some(request.expected_output.full_product_bytes)
            || self.asset.remote_validator().content_length
                != Some(request.expected_output.full_product_bytes)
        {
            return Err(Hdf5RangeTransferError::SourceChanged);
        }
        let _destination_guard = self
            .destination_lease
            .acquire_for_runtime(&self.workspace.part_path(), cancelled)
            .map_err(map_destination_lease_error)?;
        let _workspace_lock = self.workspace.acquire_lock()?;
        self.workspace.bind_plan(plan)?;
        let mut metrics = self.workspace.load_metrics(plan)?;
        let etag = frozen_strong_etag(&self.asset)?;
        let (mut part, part_identity) = open_part(
            &self.workspace.part_path(),
            request.expected_output.full_product_bytes,
        )?;
        self.workspace.bind_part_identity(plan, &part_identity)?;
        let mut manifest = match self.workspace.load_manifest()? {
            Some(manifest) => {
                manifest
                    .validate_against_request(request)
                    .map_err(|_| Hdf5RangeTransferError::RecoveryInvalid)?;
                validate_manifest_plan(&manifest, plan, &etag)?;
                verify_completed_ranges(&manifest, &mut part)?;
                reconcile_metrics_with_manifest(&mut metrics, &manifest)?;
                self.workspace.persist_metrics(plan, metrics)?;
                if manifest.state == NisarSubsetTransferState::RangesComplete {
                    return Ok(Hdf5RangeTransferResultV1 {
                        manifest,
                        metrics,
                        plan_sha256: plan.plan_sha256.clone(),
                    });
                }
                if matches!(
                    manifest.state,
                    NisarSubsetTransferState::SourceChanged | NisarSubsetTransferState::Failed
                ) {
                    // SOURCE_CHANGED is intentionally terminal for this immutable
                    // task/workspace. A newly frozen validator must use a new Store
                    // task and workspace; this adapter never clears old bytes and
                    // silently continues under a different source identity.
                    return Err(Hdf5RangeTransferError::RecoveryInvalid);
                }
                manifest
            }
            None => {
                let manifest = planned_manifest(request, plan, timestamp, &etag)?;
                persist_manifest(&self.workspace, &manifest)?;
                manifest
            }
        };
        for index in 0..manifest.ranges.len() {
            if manifest.ranges[index].state == NisarSubsetRangeState::Complete {
                continue;
            }
            if cancelled() {
                manifest.state = NisarSubsetTransferState::Interrupted;
                manifest.updated_at = timestamp.to_owned();
                persist_manifest(&self.workspace, &manifest)?;
                return Err(Hdf5RangeTransferError::Cancelled);
            }
            let start = manifest.ranges[index].start;
            let end_exclusive = manifest.ranges[index].end_exclusive;
            let expected = end_exclusive - start;
            let capacity = usize::try_from(expected)
                .map_err(|_| Hdf5RangeTransferError::RangeBytesExceeded)?;
            let mut body = Vec::with_capacity(capacity);
            let mut slice_start = start;
            while slice_start < end_exclusive {
                if cancelled() {
                    manifest.state = NisarSubsetTransferState::Interrupted;
                    manifest.updated_at = timestamp.to_owned();
                    persist_manifest(&self.workspace, &manifest)?;
                    return Err(Hdf5RangeTransferError::Cancelled);
                }
                let slice_end = slice_start
                    .saturating_add(HDF5_RANGE_NETWORK_SLICE_BYTES)
                    .min(end_exclusive);
                #[cfg(feature = "nisar-subset-production")]
                if let Some(budget) = budget {
                    budget.reserve_response_body_bytes(
                        slice_end
                            .checked_sub(slice_start)
                            .and_then(|bytes| bytes.checked_add(1))
                            .ok_or(Hdf5RangeTransferError::RangeBytesExceeded)?,
                    )?;
                }
                let outcome = {
                    #[cfg(feature = "nisar-subset-production")]
                    if let Some(budget) = budget {
                        self.http.send_resolved_exact_range_with_nisar_budget(
                            &self.asset,
                            slice_start,
                            slice_end,
                            &etag,
                            cancelled,
                            budget,
                        )
                    } else {
                        self.http.send_resolved_exact_range(
                            &self.asset,
                            slice_start,
                            slice_end,
                            &etag,
                            cancelled,
                        )
                    }
                    #[cfg(not(feature = "nisar-subset-production"))]
                    {
                        self.http.send_resolved_exact_range(
                            &self.asset,
                            slice_start,
                            slice_end,
                            &etag,
                            cancelled,
                        )
                    }
                };
                let mut response = match outcome {
                    ExactRangeSendOutcome::Response {
                        response,
                        request_count,
                    } => {
                        add_request_count(&mut metrics, request_count)?;
                        self.workspace.persist_metrics(plan, metrics)?;
                        response
                    }
                    ExactRangeSendOutcome::Cancelled { request_count } => {
                        add_request_count(&mut metrics, request_count)?;
                        self.workspace.persist_metrics(plan, metrics)?;
                        manifest.state = NisarSubsetTransferState::Interrupted;
                        manifest.updated_at = timestamp.to_owned();
                        persist_manifest(&self.workspace, &manifest)?;
                        return Err(Hdf5RangeTransferError::Cancelled);
                    }
                    ExactRangeSendOutcome::RetryableExhausted { request_count, .. } => {
                        add_request_count(&mut metrics, request_count)?;
                        self.workspace.persist_metrics(plan, metrics)?;
                        manifest.state = NisarSubsetTransferState::Interrupted;
                        manifest.updated_at = timestamp.to_owned();
                        persist_manifest(&self.workspace, &manifest)?;
                        return Err(Hdf5RangeTransferError::Transport);
                    }
                    ExactRangeSendOutcome::ProtocolRejected { request_count }
                    | ExactRangeSendOutcome::TrustedRedirectRejected { request_count, .. } => {
                        add_request_count(&mut metrics, request_count)?;
                        self.workspace.persist_metrics(plan, metrics)?;
                        mark_failed(&self.workspace, &mut manifest, index, timestamp)?;
                        return Err(Hdf5RangeTransferError::ResponseInvalid);
                    }
                    #[cfg(feature = "nisar-subset-production")]
                    ExactRangeSendOutcome::BudgetExceeded { request_count } => {
                        add_request_count(&mut metrics, request_count)?;
                        self.workspace.persist_metrics(plan, metrics)?;
                        mark_failed(&self.workspace, &mut manifest, index, timestamp)?;
                        return Err(Hdf5RangeTransferError::RangeCountExceeded);
                    }
                };
                if matches!(
                    response.status(),
                    StatusCode::UNAUTHORIZED | StatusCode::FORBIDDEN
                ) {
                    mark_failed(&self.workspace, &mut manifest, index, timestamp)?;
                    return Err(Hdf5RangeTransferError::CredentialsRequired);
                }
                let response_etag = match response_strong_etag(&response) {
                    Ok(value) => value,
                    Err(error) => {
                        mark_failed(&self.workspace, &mut manifest, index, timestamp)?;
                        return Err(error);
                    }
                };
                if response_etag != etag {
                    mark_source_changed(&self.workspace, &mut manifest, timestamp)?;
                    return Err(Hdf5RangeTransferError::SourceChanged);
                }
                if response.status() == StatusCode::OK {
                    mark_failed(&self.workspace, &mut manifest, index, timestamp)?;
                    return Err(Hdf5RangeTransferError::RangeUnsupported);
                }
                if response.status() == StatusCode::RANGE_NOT_SATISFIABLE {
                    mark_failed(&self.workspace, &mut manifest, index, timestamp)?;
                    return Err(Hdf5RangeTransferError::RangeNotSatisfiable);
                }
                if response.status() != StatusCode::PARTIAL_CONTENT {
                    mark_failed(&self.workspace, &mut manifest, index, timestamp)?;
                    return Err(Hdf5RangeTransferError::ResponseInvalid);
                }
                if let Err(error) = validate_content_range(
                    &response,
                    slice_start,
                    slice_end,
                    request.expected_output.full_product_bytes,
                ) {
                    mark_failed(&self.workspace, &mut manifest, index, timestamp)?;
                    return Err(error);
                }
                let slice_expected = slice_end - slice_start;
                let mut slice_received = 0_u64;
                let mut buffer = [0_u8; HDF5_RANGE_READ_BUFFER_BYTES];
                let mut read_error = None;
                while slice_received <= slice_expected {
                    if cancelled() {
                        read_error = Some(Hdf5RangeTransferError::Cancelled);
                        break;
                    }
                    let remaining = slice_expected + 1 - slice_received;
                    let read_limit = usize::try_from(remaining.min(buffer.len() as u64))
                        .map_err(|_| Hdf5RangeTransferError::RangeBytesExceeded)?;
                    match response.read(&mut buffer[..read_limit]) {
                        Ok(0) => break,
                        Ok(count) => {
                            body.extend_from_slice(&buffer[..count]);
                            slice_received = slice_received
                                .checked_add(count as u64)
                                .ok_or(Hdf5RangeTransferError::RangeBytesExceeded)?;
                            add_response_bytes(&mut metrics, count as u64)?;
                        }
                        Err(_) => {
                            read_error = Some(if cancelled() {
                                Hdf5RangeTransferError::Cancelled
                            } else {
                                Hdf5RangeTransferError::PartialResponse
                            });
                            break;
                        }
                    }
                }
                self.workspace.persist_metrics(plan, metrics)?;
                if let Some(error) = read_error {
                    manifest.state = NisarSubsetTransferState::Interrupted;
                    manifest.updated_at = timestamp.to_owned();
                    persist_manifest(&self.workspace, &manifest)?;
                    return Err(error);
                }
                if slice_received != slice_expected || cancelled() {
                    manifest.state = NisarSubsetTransferState::Interrupted;
                    manifest.updated_at = timestamp.to_owned();
                    persist_manifest(&self.workspace, &manifest)?;
                    return Err(if cancelled() {
                        Hdf5RangeTransferError::Cancelled
                    } else {
                        Hdf5RangeTransferError::PartialResponse
                    });
                }
                slice_start = slice_end;
            }
            if body.len() as u64 != expected {
                return Err(Hdf5RangeTransferError::PartialResponse);
            }
            part.seek(SeekFrom::Start(start))
                .and_then(|_| part.write_all(&body))
                .and_then(|_| part.sync_data())
                .map_err(|_| Hdf5RangeTransferError::Persistence)?;
            if cancelled() {
                manifest.state = NisarSubsetTransferState::Interrupted;
                manifest.updated_at = timestamp.to_owned();
                persist_manifest(&self.workspace, &manifest)?;
                return Err(Hdf5RangeTransferError::Cancelled);
            }
            let digest = sha256(&body);
            manifest.ranges[index].received_bytes = expected;
            manifest.ranges[index].sha256 = Some(digest);
            manifest.ranges[index].state = NisarSubsetRangeState::Complete;
            manifest.received_bytes = manifest
                .ranges
                .iter()
                .map(|range| range.received_bytes)
                .sum();
            manifest.updated_at = timestamp.to_owned();
            manifest.state = if manifest
                .ranges
                .iter()
                .all(|range| range.state == NisarSubsetRangeState::Complete)
            {
                NisarSubsetTransferState::RangesComplete
            } else {
                NisarSubsetTransferState::InProgress
            };
            manifest
                .validate()
                .map_err(|_| Hdf5RangeTransferError::Persistence)?;
            persist_manifest(&self.workspace, &manifest)?;
            metrics.accepted_unique_bytes = metrics
                .accepted_unique_bytes
                .checked_add(expected)
                .ok_or(Hdf5RangeTransferError::RangeBytesExceeded)?;
            metrics.retry_duplicate_bytes = metrics
                .response_body_bytes
                .saturating_sub(metrics.accepted_unique_bytes);
            self.workspace.persist_metrics(plan, metrics)?;
        }
        if manifest.state != NisarSubsetTransferState::RangesComplete {
            return Err(Hdf5RangeTransferError::RecoveryInvalid);
        }
        Ok(Hdf5RangeTransferResultV1 {
            manifest,
            metrics,
            plan_sha256: plan.plan_sha256.clone(),
        })
    }
}

fn planned_manifest(
    request: &NisarSubsetRequestManifestV1,
    plan: &Hdf5RangePlanV1,
    timestamp: &str,
    etag: &str,
) -> Result<NisarSubsetTransferManifestV1, Hdf5RangeTransferError> {
    let manifest = NisarSubsetTransferManifestV1 {
        manifest_contract: insar_core::NISAR_SUBSET_TRANSFER_MANIFEST_CONTRACT.to_owned(),
        schema_version: insar_core::NISAR_SUBSET_SCHEMA_VERSION,
        request_manifest_sha256: plan.request_manifest_sha256.clone(),
        source_identity: request.source.clone(),
        source_validator: NisarSubsetSourceValidatorV1 {
            kind: NisarSubsetValidatorKind::StrongEtag,
            value: etag.to_owned(),
            content_length: request.expected_output.full_product_bytes,
        },
        ranges: plan
            .ranges
            .iter()
            .map(|range| NisarSubsetByteRangeV1 {
                start: range.start,
                end_exclusive: range.end_exclusive,
                expected_bytes: range.end_exclusive - range.start,
                received_bytes: 0,
                sha256: None,
                state: NisarSubsetRangeState::Planned,
            })
            .collect(),
        expected_remote_bytes: plan.expected_remote_bytes,
        received_bytes: 0,
        state: NisarSubsetTransferState::Planned,
        created_at: timestamp.to_owned(),
        updated_at: timestamp.to_owned(),
    };
    manifest
        .validate_against_request(request)
        .map_err(|_| Hdf5RangeTransferError::RangeInvalid)?;
    Ok(manifest)
}

fn validate_manifest_plan(
    manifest: &NisarSubsetTransferManifestV1,
    plan: &Hdf5RangePlanV1,
    etag: &str,
) -> Result<(), Hdf5RangeTransferError> {
    if manifest.request_manifest_sha256 != plan.request_manifest_sha256
        || manifest.expected_remote_bytes != plan.expected_remote_bytes
        || manifest.source_validator.kind != NisarSubsetValidatorKind::StrongEtag
        || manifest.source_validator.value != etag
        || manifest.ranges.len() != plan.ranges.len()
        || manifest
            .ranges
            .iter()
            .zip(&plan.ranges)
            .any(|(left, right)| {
                left.start != right.start
                    || left.end_exclusive != right.end_exclusive
                    || left.expected_bytes != right.end_exclusive - right.start
            })
    {
        return Err(Hdf5RangeTransferError::RecoveryInvalid);
    }
    Ok(())
}

fn verify_completed_ranges(
    manifest: &NisarSubsetTransferManifestV1,
    file: &mut File,
) -> Result<(), Hdf5RangeTransferError> {
    if !manifest
        .ranges
        .iter()
        .any(|range| range.state == NisarSubsetRangeState::Complete)
    {
        return Ok(());
    }
    for range in manifest
        .ranges
        .iter()
        .filter(|range| range.state == NisarSubsetRangeState::Complete)
    {
        let mut bytes = vec![0_u8; range.expected_bytes as usize];
        file.seek(SeekFrom::Start(range.start))
            .and_then(|_| file.read_exact(&mut bytes))
            .map_err(|_| Hdf5RangeTransferError::RecoveryInvalid)?;
        if range.sha256.as_deref() != Some(sha256(&bytes).as_str()) {
            return Err(Hdf5RangeTransferError::RecoveryInvalid);
        }
    }
    Ok(())
}

fn open_part(
    path: &Path,
    source_length: u64,
) -> Result<(File, PartFileIdentityV1), Hdf5RangeTransferError> {
    let (file, created) = open_part_file(path).map_err(|_| Hdf5RangeTransferError::Persistence)?;
    let identity = part_file_identity(&file)?;
    let length = file
        .metadata()
        .map_err(|_| Hdf5RangeTransferError::Persistence)?
        .len();
    if !created && length != source_length {
        return Err(Hdf5RangeTransferError::RecoveryInvalid);
    }
    if created {
        file.set_len(source_length)
            .and_then(|_| file.sync_all())
            .map_err(|_| Hdf5RangeTransferError::Persistence)?;
    }
    Ok((file, identity))
}

#[cfg(windows)]
fn open_existing_part_file(path: &Path) -> std::io::Result<File> {
    use std::os::windows::fs::OpenOptionsExt;

    OpenOptions::new()
        .read(true)
        .write(true)
        .share_mode(0x0000_0001)
        .custom_flags(0x0020_0000)
        .open(path)
}

#[cfg(any(
    target_os = "linux",
    target_os = "android",
    target_os = "solaris",
    target_os = "illumos",
    target_os = "macos",
    target_os = "ios",
    target_os = "freebsd",
    target_os = "dragonfly",
    target_os = "netbsd",
    target_os = "openbsd"
))]
fn open_existing_part_file(path: &Path) -> std::io::Result<File> {
    use std::os::unix::fs::OpenOptionsExt;

    OpenOptions::new()
        .read(true)
        .write(true)
        .custom_flags(UNIX_O_NOFOLLOW)
        .open(path)
}

#[cfg(all(
    unix,
    not(any(
        target_os = "linux",
        target_os = "android",
        target_os = "solaris",
        target_os = "illumos",
        target_os = "macos",
        target_os = "ios",
        target_os = "freebsd",
        target_os = "dragonfly",
        target_os = "netbsd",
        target_os = "openbsd"
    ))
))]
fn open_existing_part_file(_path: &Path) -> std::io::Result<File> {
    Err(std::io::Error::new(
        std::io::ErrorKind::Unsupported,
        "no audited no-follow open flag for this Unix target",
    ))
}

#[cfg(all(not(windows), not(unix)))]
fn open_existing_part_file(_path: &Path) -> std::io::Result<File> {
    Err(std::io::Error::new(
        std::io::ErrorKind::Unsupported,
        "safe part-file opening is unsupported on this target",
    ))
}

#[cfg(windows)]
fn open_existing_part_for_writer(path: &Path) -> std::io::Result<File> {
    use std::os::windows::fs::OpenOptionsExt;

    OpenOptions::new()
        .read(true)
        // HDF5 may obtain additional read handles. Denying write and delete
        // sharing pins both the pathname and material for the Writer lifetime.
        .share_mode(0x0000_0001)
        .custom_flags(0x0020_0000)
        .open(path)
}

#[cfg(any(
    target_os = "linux",
    target_os = "android",
    target_os = "solaris",
    target_os = "illumos",
    target_os = "macos",
    target_os = "ios",
    target_os = "freebsd",
    target_os = "dragonfly",
    target_os = "netbsd",
    target_os = "openbsd"
))]
fn open_existing_part_for_writer(path: &Path) -> std::io::Result<File> {
    use std::os::unix::fs::OpenOptionsExt;

    OpenOptions::new()
        .read(true)
        .custom_flags(UNIX_O_NOFOLLOW)
        .open(path)
}

#[cfg(all(
    unix,
    not(any(
        target_os = "linux",
        target_os = "android",
        target_os = "solaris",
        target_os = "illumos",
        target_os = "macos",
        target_os = "ios",
        target_os = "freebsd",
        target_os = "dragonfly",
        target_os = "netbsd",
        target_os = "openbsd"
    ))
))]
fn open_existing_part_for_writer(_path: &Path) -> std::io::Result<File> {
    Err(std::io::Error::new(
        std::io::ErrorKind::Unsupported,
        "no audited no-follow Writer open flag for this Unix target",
    ))
}

#[cfg(all(not(windows), not(unix)))]
fn open_existing_part_for_writer(_path: &Path) -> std::io::Result<File> {
    Err(std::io::Error::new(
        std::io::ErrorKind::Unsupported,
        "safe Writer part-file opening is unsupported on this target",
    ))
}

#[cfg(windows)]
fn writer_part_mutation_marker(file: &File) -> Result<String, Hdf5RangeTransferError> {
    use std::os::windows::fs::MetadataExt;

    let metadata = file
        .metadata()
        .map_err(|_| Hdf5RangeTransferError::RecoveryInvalid)?;
    Ok(sha256(
        format!(
            "windows:{}:{}:{}:{}",
            metadata.file_size(),
            metadata.creation_time(),
            metadata.last_write_time(),
            metadata.file_attributes()
        )
        .as_bytes(),
    ))
}

#[cfg(unix)]
fn writer_part_mutation_marker(file: &File) -> Result<String, Hdf5RangeTransferError> {
    use std::os::unix::fs::MetadataExt;

    let metadata = file
        .metadata()
        .map_err(|_| Hdf5RangeTransferError::RecoveryInvalid)?;
    if !metadata.is_file() || metadata.nlink() != 1 {
        return Err(Hdf5RangeTransferError::RecoveryInvalid);
    }
    Ok(sha256(
        format!(
            "unix:{}:{}:{}:{}:{}:{}:{}:{}",
            metadata.dev(),
            metadata.ino(),
            metadata.len(),
            metadata.nlink(),
            metadata.ctime(),
            metadata.ctime_nsec(),
            metadata.mtime(),
            metadata.mtime_nsec()
        )
        .as_bytes(),
    ))
}

#[cfg(all(not(windows), not(unix)))]
fn writer_part_mutation_marker(_file: &File) -> Result<String, Hdf5RangeTransferError> {
    Err(Hdf5RangeTransferError::WorkspaceInvalid)
}

#[cfg(windows)]
fn immutable_writer_part_view(
    source: &File,
    source_path: &Path,
    _snapshot_path: &Path,
) -> Result<(File, PathBuf), Hdf5RangeTransferError> {
    let view = source
        .try_clone()
        .map_err(|_| Hdf5RangeTransferError::RecoveryInvalid)?;
    Ok((view, source_path.to_path_buf()))
}

/// Linux/Android read the pinned source itself through its descriptor, as
/// Windows does (owner decision 2026-11-02). HDF5 opens `/proc/self/fd/<n>`,
/// which names this open file rather than a swappable pathname, and resolves
/// it to the part's real path. A write to the part changes its change time,
/// which the source mutation marker pins, and both material digests read this
/// view. (A reflinked fs-verity copy was unusable here: HDF5 cannot resolve
/// the real path of an unnamed file, and ext4 has no reflink.)
#[cfg(any(target_os = "linux", target_os = "android"))]
fn immutable_writer_part_view(
    source: &File,
    _source_path: &Path,
    _snapshot_path: &Path,
) -> Result<(File, PathBuf), Hdf5RangeTransferError> {
    use std::os::fd::AsRawFd;

    let view = source
        .try_clone()
        .map_err(|_| Hdf5RangeTransferError::RecoveryInvalid)?;
    let descriptor_path = PathBuf::from(format!("/proc/self/fd/{}", view.as_raw_fd()));
    Ok((view, descriptor_path))
}

#[cfg(target_os = "macos")]
fn immutable_writer_part_view(
    source: &File,
    _source_path: &Path,
    _snapshot_path: &Path,
) -> Result<(File, PathBuf), Hdf5RangeTransferError> {
    use std::os::fd::AsRawFd;

    let view = source
        .try_clone()
        .map_err(|_| Hdf5RangeTransferError::RecoveryInvalid)?;
    let descriptor_path = PathBuf::from(format!("/dev/fd/{}", view.as_raw_fd()));
    Ok((view, descriptor_path))
}

/// The filesystem has no fs-verity, or it is not enabled on it.
#[cfg(any(target_os = "linux", target_os = "android"))]
pub(crate) fn verity_unsupported(error: &std::io::Error) -> bool {
    matches!(
        error.raw_os_error(),
        Some(libc::EOPNOTSUPP | libc::ENOTTY | libc::ENOSYS)
    )
}

#[cfg(all(
    unix,
    not(any(target_os = "linux", target_os = "android", target_os = "macos"))
))]
fn immutable_writer_part_view(
    _source: &File,
    _source_path: &Path,
    _snapshot_path: &Path,
) -> Result<(File, PathBuf), Hdf5RangeTransferError> {
    Err(Hdf5RangeTransferError::WorkspaceInvalid)
}

#[cfg(all(not(windows), not(unix)))]
fn immutable_writer_part_view(
    _source: &File,
    _source_path: &Path,
    _snapshot_path: &Path,
) -> Result<(File, PathBuf), Hdf5RangeTransferError> {
    Err(Hdf5RangeTransferError::WorkspaceInvalid)
}

#[cfg(windows)]
fn open_part_file(path: &Path) -> std::io::Result<(File, bool)> {
    use std::os::windows::fs::OpenOptionsExt;

    let created = OpenOptions::new()
        .create_new(true)
        .read(true)
        .write(true)
        .share_mode(0)
        .custom_flags(0x0020_0000)
        .open(path);
    match created {
        Ok(file) => Ok((file, true)),
        Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => OpenOptions::new()
            .read(true)
            .write(true)
            .share_mode(0)
            .custom_flags(0x0020_0000)
            .open(path)
            .map(|file| (file, false)),
        Err(error) => Err(error),
    }
}

#[cfg(any(
    target_os = "linux",
    target_os = "android",
    target_os = "solaris",
    target_os = "illumos",
    target_os = "macos",
    target_os = "ios",
    target_os = "freebsd",
    target_os = "dragonfly",
    target_os = "netbsd",
    target_os = "openbsd"
))]
fn open_part_file(path: &Path) -> std::io::Result<(File, bool)> {
    use std::os::unix::fs::OpenOptionsExt;

    let created = OpenOptions::new()
        .create_new(true)
        .read(true)
        .write(true)
        .custom_flags(UNIX_O_NOFOLLOW)
        .open(path);
    match created {
        Ok(file) => Ok((file, true)),
        Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => OpenOptions::new()
            .read(true)
            .write(true)
            .custom_flags(UNIX_O_NOFOLLOW)
            .open(path)
            .map(|file| (file, false)),
        Err(error) => Err(error),
    }
}

#[cfg(all(
    unix,
    not(any(
        target_os = "linux",
        target_os = "android",
        target_os = "solaris",
        target_os = "illumos",
        target_os = "macos",
        target_os = "ios",
        target_os = "freebsd",
        target_os = "dragonfly",
        target_os = "netbsd",
        target_os = "openbsd"
    ))
))]
fn open_part_file(_path: &Path) -> std::io::Result<(File, bool)> {
    Err(std::io::Error::new(
        std::io::ErrorKind::Unsupported,
        "no audited no-follow open flag for this Unix target",
    ))
}

#[cfg(all(not(windows), not(unix)))]
fn open_part_file(_path: &Path) -> std::io::Result<(File, bool)> {
    Err(std::io::Error::new(
        std::io::ErrorKind::Unsupported,
        "safe part-file opening is unsupported on this target",
    ))
}

#[cfg(windows)]
fn part_file_identity(file: &File) -> Result<PartFileIdentityV1, Hdf5RangeTransferError> {
    use std::os::windows::io::AsRawHandle;
    use windows_sys::Win32::Storage::FileSystem::{
        GetFileInformationByHandle, BY_HANDLE_FILE_INFORMATION,
    };

    // SAFETY: `file` owns a live Windows file handle for this call and `info`
    // points to writable storage of the exact Win32 output type.
    let mut info = unsafe { std::mem::zeroed::<BY_HANDLE_FILE_INFORMATION>() };
    // SAFETY: the handle remains valid for the duration of the call and the
    // mutable output pointer is exclusive.
    let succeeded = unsafe { GetFileInformationByHandle(file.as_raw_handle() as _, &mut info) };
    if succeeded == 0 || info.dwFileAttributes & 0x0000_0400 != 0 || info.nNumberOfLinks != 1 {
        return Err(Hdf5RangeTransferError::RecoveryInvalid);
    }
    Ok(PartFileIdentityV1 {
        platform: "windows".to_owned(),
        device_id: u64::from(info.dwVolumeSerialNumber),
        file_id: (u64::from(info.nFileIndexHigh) << 32) | u64::from(info.nFileIndexLow),
        birth_ns: None,
    })
}

#[cfg(unix)]
fn part_file_identity(file: &File) -> Result<PartFileIdentityV1, Hdf5RangeTransferError> {
    use std::os::unix::fs::MetadataExt;

    let metadata = file
        .metadata()
        .map_err(|_| Hdf5RangeTransferError::Persistence)?;
    if !metadata.is_file() || metadata.nlink() != 1 {
        return Err(Hdf5RangeTransferError::RecoveryInvalid);
    }
    let birth_ns = metadata
        .created()
        .ok()
        .and_then(|created| created.duration_since(std::time::UNIX_EPOCH).ok())
        .and_then(|age| u64::try_from(age.as_nanos()).ok());
    Ok(PartFileIdentityV1 {
        platform: "unix".to_owned(),
        device_id: metadata.dev(),
        file_id: metadata.ino(),
        birth_ns,
    })
}

#[cfg(all(not(windows), not(unix)))]
fn part_file_identity(_file: &File) -> Result<PartFileIdentityV1, Hdf5RangeTransferError> {
    Err(Hdf5RangeTransferError::WorkspaceInvalid)
}

pub(crate) fn guarded_regular_file_sha256_v2(
    path: &Path,
) -> Result<(String, u64), Hdf5RangeTransferError> {
    let mut file =
        open_checkpoint_nofollow(path).map_err(|_| Hdf5RangeTransferError::RecoveryInvalid)?;
    part_file_identity(&file).map_err(|_| Hdf5RangeTransferError::RecoveryInvalid)?;
    let length = file
        .metadata()
        .map_err(|_| Hdf5RangeTransferError::RecoveryInvalid)?
        .len();
    if length == 0 {
        return Err(Hdf5RangeTransferError::RecoveryInvalid);
    }
    let mut digest = Sha256::new();
    let mut total = 0_u64;
    let mut buffer = [0_u8; HDF5_RANGE_READ_BUFFER_BYTES];
    loop {
        let count = file
            .read(&mut buffer)
            .map_err(|_| Hdf5RangeTransferError::RecoveryInvalid)?;
        if count == 0 {
            break;
        }
        total = total
            .checked_add(count as u64)
            .ok_or(Hdf5RangeTransferError::RecoveryInvalid)?;
        if total > length {
            return Err(Hdf5RangeTransferError::RecoveryInvalid);
        }
        digest.update(&buffer[..count]);
    }
    if total != length {
        return Err(Hdf5RangeTransferError::RecoveryInvalid);
    }
    Ok((format!("{:x}", digest.finalize()), length))
}

fn read_guarded_checkpoint(
    path: &Path,
    maximum_bytes: u64,
) -> Result<Option<String>, Hdf5RangeTransferError> {
    let mut file = match open_checkpoint_nofollow(path) {
        Ok(file) => file,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(_) => return Err(Hdf5RangeTransferError::RecoveryInvalid),
    };
    part_file_identity(&file).map_err(|_| Hdf5RangeTransferError::RecoveryInvalid)?;
    let length = file
        .metadata()
        .map_err(|_| Hdf5RangeTransferError::RecoveryInvalid)?
        .len();
    if length > maximum_bytes {
        return Err(Hdf5RangeTransferError::RecoveryInvalid);
    }
    let capacity = usize::try_from(length).map_err(|_| Hdf5RangeTransferError::RecoveryInvalid)?;
    let mut bytes = Vec::with_capacity(capacity);
    Read::by_ref(&mut file)
        .take(maximum_bytes.saturating_add(1))
        .read_to_end(&mut bytes)
        .map_err(|_| Hdf5RangeTransferError::RecoveryInvalid)?;
    if bytes.len() != capacity || bytes.len() as u64 > maximum_bytes {
        return Err(Hdf5RangeTransferError::RecoveryInvalid);
    }
    String::from_utf8(bytes)
        .map(Some)
        .map_err(|_| Hdf5RangeTransferError::RecoveryInvalid)
}

#[cfg(windows)]
fn open_checkpoint_nofollow(path: &Path) -> std::io::Result<File> {
    use std::os::windows::fs::OpenOptionsExt;

    OpenOptions::new()
        .read(true)
        .share_mode(0x0000_0001)
        .custom_flags(0x0020_0000)
        .open(path)
}

#[cfg(any(
    target_os = "linux",
    target_os = "android",
    target_os = "solaris",
    target_os = "illumos",
    target_os = "macos",
    target_os = "ios",
    target_os = "freebsd",
    target_os = "dragonfly",
    target_os = "netbsd",
    target_os = "openbsd"
))]
fn open_checkpoint_nofollow(path: &Path) -> std::io::Result<File> {
    use std::os::unix::fs::OpenOptionsExt;

    OpenOptions::new()
        .read(true)
        .custom_flags(UNIX_O_NOFOLLOW | checkpoint_o_nonblock())
        .open(path)
}

#[cfg(any(target_os = "linux", target_os = "android"))]
const fn checkpoint_o_nonblock() -> i32 {
    0x0000_0800
}

#[cfg(any(target_os = "solaris", target_os = "illumos"))]
const fn checkpoint_o_nonblock() -> i32 {
    0x0000_0080
}

#[cfg(any(
    target_os = "macos",
    target_os = "ios",
    target_os = "freebsd",
    target_os = "dragonfly",
    target_os = "netbsd",
    target_os = "openbsd"
))]
const fn checkpoint_o_nonblock() -> i32 {
    0x0000_0004
}

#[cfg(all(
    unix,
    not(any(
        target_os = "linux",
        target_os = "android",
        target_os = "solaris",
        target_os = "illumos",
        target_os = "macos",
        target_os = "ios",
        target_os = "freebsd",
        target_os = "dragonfly",
        target_os = "netbsd",
        target_os = "openbsd"
    ))
))]
fn open_checkpoint_nofollow(_path: &Path) -> std::io::Result<File> {
    Err(std::io::Error::new(
        std::io::ErrorKind::Unsupported,
        "safe checkpoint opening is unsupported on this Unix target",
    ))
}

#[cfg(all(not(windows), not(unix)))]
fn open_checkpoint_nofollow(_path: &Path) -> std::io::Result<File> {
    Err(std::io::Error::new(
        std::io::ErrorKind::Unsupported,
        "safe checkpoint opening is unsupported on this platform",
    ))
}

fn persist_manifest(
    workspace: &Hdf5RangeWorkspaceV1,
    manifest: &NisarSubsetTransferManifestV1,
) -> Result<(), Hdf5RangeTransferError> {
    let canonical = manifest
        .canonical_json()
        .map_err(|_| Hdf5RangeTransferError::Persistence)?;
    persist_atomic_text(
        workspace,
        &workspace.next_manifest_path(),
        &workspace.manifest_path(),
        &canonical,
    )
}

fn persist_atomic_text(
    workspace: &Hdf5RangeWorkspaceV1,
    next: &Path,
    destination: &Path,
    canonical: &str,
) -> Result<(), Hdf5RangeTransferError> {
    workspace
        .root
        .verify()
        .map_err(|_| Hdf5RangeTransferError::WorkspaceInvalid)?;
    if next.exists() {
        if !next.is_file() {
            return Err(Hdf5RangeTransferError::Persistence);
        }
        fs::remove_file(next).map_err(|_| Hdf5RangeTransferError::Persistence)?;
    }
    let mut file = OpenOptions::new()
        .create_new(true)
        .write(true)
        .open(next)
        .map_err(|_| Hdf5RangeTransferError::Persistence)?;
    file.write_all(canonical.as_bytes())
        .and_then(|_| file.sync_all())
        .map_err(|_| Hdf5RangeTransferError::Persistence)?;
    drop(file);
    promote_verified_closed_artifact(
        VerifiedClosedArtifact::after_scientific_validation(
            next.to_path_buf(),
            sha256(canonical.as_bytes()),
            &|| false,
        )
        .map_err(|_| Hdf5RangeTransferError::Persistence)?,
        destination,
        None,
    )
    .map_err(|_| Hdf5RangeTransferError::Persistence)?;
    workspace
        .root
        .verify()
        .map_err(|_| Hdf5RangeTransferError::WorkspaceInvalid)?;
    Ok(())
}

fn mark_source_changed(
    workspace: &Hdf5RangeWorkspaceV1,
    manifest: &mut NisarSubsetTransferManifestV1,
    timestamp: &str,
) -> Result<(), Hdf5RangeTransferError> {
    manifest.state = NisarSubsetTransferState::SourceChanged;
    manifest.updated_at = timestamp.to_owned();
    persist_manifest(workspace, manifest)
}

fn mark_failed(
    workspace: &Hdf5RangeWorkspaceV1,
    manifest: &mut NisarSubsetTransferManifestV1,
    index: usize,
    timestamp: &str,
) -> Result<(), Hdf5RangeTransferError> {
    manifest.ranges[index].state = NisarSubsetRangeState::Failed;
    manifest.state = NisarSubsetTransferState::Failed;
    manifest.updated_at = timestamp.to_owned();
    persist_manifest(workspace, manifest)
}

fn map_destination_lease_error(error: DestinationLeaseError) -> Hdf5RangeTransferError {
    match error {
        DestinationLeaseError::Cancelled => Hdf5RangeTransferError::Cancelled,
        DestinationLeaseError::InvalidIdentity
        | DestinationLeaseError::Unavailable
        | DestinationLeaseError::InvariantViolation => Hdf5RangeTransferError::WorkspaceInvalid,
    }
}

fn add_request_count(
    metrics: &mut Hdf5RangeTransferMetricsV1,
    request_count: u64,
) -> Result<(), Hdf5RangeTransferError> {
    metrics.request_count = metrics
        .request_count
        .checked_add(request_count)
        .ok_or(Hdf5RangeTransferError::RangeCountExceeded)?;
    #[cfg(feature = "nisar-subset-production")]
    validate_nisar_live_acceptance_budget(metrics.request_count, metrics.response_body_bytes)?;
    Ok(())
}

fn add_response_bytes(
    metrics: &mut Hdf5RangeTransferMetricsV1,
    response_bytes: u64,
) -> Result<(), Hdf5RangeTransferError> {
    metrics.response_body_bytes = metrics
        .response_body_bytes
        .checked_add(response_bytes)
        .ok_or(Hdf5RangeTransferError::RangeBytesExceeded)?;
    #[cfg(feature = "nisar-subset-production")]
    validate_nisar_live_acceptance_budget(metrics.request_count, metrics.response_body_bytes)?;
    metrics.retry_duplicate_bytes = metrics
        .response_body_bytes
        .saturating_sub(metrics.accepted_unique_bytes);
    Ok(())
}

fn reconcile_metrics_with_manifest(
    metrics: &mut Hdf5RangeTransferMetricsV1,
    manifest: &NisarSubsetTransferManifestV1,
) -> Result<(), Hdf5RangeTransferError> {
    if manifest.received_bytes > metrics.response_body_bytes {
        return Err(Hdf5RangeTransferError::RecoveryInvalid);
    }
    metrics.accepted_unique_bytes = manifest.received_bytes;
    metrics.retry_duplicate_bytes = metrics.response_body_bytes - manifest.received_bytes;
    Ok(())
}

fn resolved_checksum_matches_request(
    asset: &ResolvedAsset,
    request: &NisarSubsetRequestManifestV1,
) -> bool {
    match (&request.source.checksum, asset.expected_checksum()) {
        (None, None) => true,
        (Some(expected), Some(actual)) => {
            let algorithm = match expected.algorithm {
                NisarSubsetChecksumAlgorithm::Md5 => "md5",
                NisarSubsetChecksumAlgorithm::Sha1 => "sha1",
                NisarSubsetChecksumAlgorithm::Sha256 => "sha256",
            };
            actual.algorithm.eq_ignore_ascii_case(algorithm)
                && actual.value.eq_ignore_ascii_case(&expected.value)
        }
        _ => false,
    }
}

fn frozen_strong_etag(asset: &ResolvedAsset) -> Result<String, Hdf5RangeTransferError> {
    asset
        .remote_validator()
        .etag
        .as_deref()
        .and_then(strong_etag)
        .map(str::to_owned)
        .ok_or(Hdf5RangeTransferError::StrongValidatorRequired)
}

pub(crate) fn response_strong_etag(
    response: &reqwest::blocking::Response,
) -> Result<String, Hdf5RangeTransferError> {
    let value =
        single_header_text(response, &ETAG)?.ok_or(Hdf5RangeTransferError::ResponseInvalid)?;
    if value.starts_with("W/") {
        return Err(Hdf5RangeTransferError::WeakEtagRejected);
    }
    strong_etag(value)
        .map(str::to_owned)
        .ok_or(Hdf5RangeTransferError::ResponseInvalid)
}

fn strong_etag(value: &str) -> Option<&str> {
    let value = value.trim();
    (value.len() >= 2
        && value.len() <= 1024
        && value.starts_with('"')
        && value.ends_with('"')
        && !value.starts_with("W/")
        && value[1..value.len() - 1]
            .bytes()
            .all(|byte| byte >= 0x21 && byte != 0x22 && byte != 0x7f))
    .then_some(value)
}

pub(crate) fn validate_content_range(
    response: &reqwest::blocking::Response,
    start: u64,
    end_exclusive: u64,
    total: u64,
) -> Result<(), Hdf5RangeTransferError> {
    let expected = format!("bytes {start}-{}/{total}", end_exclusive - 1);
    let range_matches = single_header_text(response, &CONTENT_RANGE)? == Some(expected.as_str());
    let length_matches = single_header_text(response, &CONTENT_LENGTH)?
        .and_then(|value| value.parse::<u64>().ok())
        == Some(end_exclusive - start);
    let encoding_matches = single_header_text(response, &CONTENT_ENCODING)?
        .is_none_or(|value| value.eq_ignore_ascii_case("identity"));
    if !range_matches || !length_matches || !encoding_matches {
        return Err(Hdf5RangeTransferError::MalformedContentRange);
    }
    Ok(())
}

fn single_header_text<'a>(
    response: &'a reqwest::blocking::Response,
    name: &HeaderName,
) -> Result<Option<&'a str>, Hdf5RangeTransferError> {
    let mut values = response.headers().get_all(name).iter();
    let Some(first) = values.next() else {
        return Ok(None);
    };
    if values.next().is_some() {
        return Err(Hdf5RangeTransferError::ResponseInvalid);
    }
    first
        .to_str()
        .map(Some)
        .map_err(|_| Hdf5RangeTransferError::ResponseInvalid)
}

fn loopback_url(value: &reqwest::Url) -> bool {
    matches!(value.scheme(), "http" | "https")
        && value.username().is_empty()
        && value.password().is_none()
        && value.query().is_none()
        && value.fragment().is_none()
        && value
            .host_str()
            .and_then(|host| host.parse::<std::net::IpAddr>().ok())
            .is_some_and(|ip| ip.is_loopback())
}

fn range_asset_url_allowed(asset: &ResolvedAsset) -> bool {
    #[cfg(any(test, feature = "offline-nisar-subset-test-support"))]
    if loopback_url(asset.effective_url()) {
        return true;
    }
    #[cfg(feature = "nisar-subset-production")]
    {
        asset
            .host_policy()
            .is_some_and(|policy| policy.allows_response_url(asset.effective_url()))
    }
    #[cfg(not(feature = "nisar-subset-production"))]
    {
        let _ = asset;
        false
    }
}

fn sha256(value: &[u8]) -> String {
    format!("{:x}", Sha256::digest(value))
}

/// Parse the canonical IMF-fixdate form required for a persisted If-Range
/// validator. Obsolete HTTP-date forms and weekday/date mismatches fail closed.
pub(crate) fn parse_canonical_http_date(value: &str) -> Option<String> {
    if value.len() != 29 || !value.is_ascii() {
        return None;
    }
    let bytes = value.as_bytes();
    if bytes[3] != b','
        || bytes[4] != b' '
        || bytes[7] != b' '
        || bytes[11] != b' '
        || bytes[16] != b' '
        || bytes[19] != b':'
        || bytes[22] != b':'
        || &value[25..] != " GMT"
    {
        return None;
    }
    let weekday = match &value[..3] {
        "Sun" => 0,
        "Mon" => 1,
        "Tue" => 2,
        "Wed" => 3,
        "Thu" => 4,
        "Fri" => 5,
        "Sat" => 6,
        _ => return None,
    };
    let day = parse_u32(&value[5..7])?;
    let month = match &value[8..11] {
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
    let year = parse_u32(&value[12..16])?;
    let hour = parse_u32(&value[17..19])?;
    let minute = parse_u32(&value[20..22])?;
    let second = parse_u32(&value[23..25])?;
    if year == 0
        || day == 0
        || day > days_in_month(year, month)
        || hour > 23
        || minute > 59
        || second > 59
    {
        return None;
    }
    let days = days_before_year(year) + u64::from(days_before_month(year, month) + day - 1);
    if (days + 1) % 7 != weekday {
        return None;
    }
    Some(value.to_owned())
}

fn parse_u32(value: &str) -> Option<u32> {
    value
        .bytes()
        .all(|byte| byte.is_ascii_digit())
        .then(|| value.parse().ok())
        .flatten()
}

fn days_before_year(year: u32) -> u64 {
    let previous = u64::from(year - 1);
    previous * 365 + previous / 4 - previous / 100 + previous / 400
}

fn days_before_month(year: u32, month: u32) -> u32 {
    const DAYS: [u32; 12] = [0, 31, 59, 90, 120, 151, 181, 212, 243, 273, 304, 334];
    DAYS[(month - 1) as usize] + u32::from(month > 2 && is_leap_year(year))
}

fn days_in_month(year: u32, month: u32) -> u32 {
    match month {
        2 if is_leap_year(year) => 29,
        2 => 28,
        4 | 6 | 9 | 11 => 30,
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        _ => 0,
    }
}

fn is_leap_year(year: u32) -> bool {
    year.is_multiple_of(400) || (year.is_multiple_of(4) && !year.is_multiple_of(100))
}

#[cfg(test)]
#[path = "hdf5_range_transfer_tests.rs"]
mod tests;

#[cfg(test)]
mod coverage_tests {
    use super::{interval_is_fully_covered, normalize_verified_ranges, VerifiedCoverage};

    fn coverage(pairs: &[(u64, u64)]) -> Vec<VerifiedCoverage> {
        normalize_verified_ranges(pairs, 10_000).expect("coverage normalizes")
    }

    #[test]
    fn straddling_chunk_is_covered_by_prefix_plus_science_union() {
        // Contract example: prefix [0,150) + science [150,220) covers [100,200).
        let merged = coverage(&[(0, 150), (150, 220)]);
        assert_eq!(merged.len(), 1);
        assert_eq!(merged[0].start, 0);
        assert_eq!(merged[0].end_exclusive, 220);
        assert!(interval_is_fully_covered((100, 200), &merged));
    }

    #[test]
    fn sparse_hole_between_ranges_fails_closed() {
        // Contract example: [0,140) + [150,220) leaves [140,150) uncovered.
        let merged = coverage(&[(0, 140), (150, 220)]);
        assert_eq!(merged.len(), 2);
        assert!(!interval_is_fully_covered((100, 200), &merged));
    }

    #[test]
    fn prefix_resident_chunk_is_covered_without_a_dedicated_science_range() {
        let merged = coverage(&[(0, 1000)]);
        assert!(interval_is_fully_covered((100, 200), &merged));
    }

    #[test]
    fn unordered_and_adjacent_ranges_normalize_deterministically() {
        let merged = coverage(&[(300, 400), (100, 200), (200, 300), (150, 250)]);
        assert_eq!(
            merged,
            vec![VerifiedCoverage {
                start: 100,
                end_exclusive: 400
            }]
        );
    }

    #[test]
    fn zero_width_or_out_of_bounds_intervals_are_rejected() {
        assert!(normalize_verified_ranges(&[(5, 5)], 10_000).is_err());
        assert!(normalize_verified_ranges(&[(5, 11)], 10).is_err());
        assert!(normalize_verified_ranges(&[(9_999, 10_000)], 10_000).is_ok());
    }

    #[test]
    fn empty_or_absent_coverage_never_covers_an_interval() {
        assert!(!interval_is_fully_covered((0, 10), &[]));
        assert!(!interval_is_fully_covered((0, 0), &coverage(&[(0, 10)])));
    }
}
