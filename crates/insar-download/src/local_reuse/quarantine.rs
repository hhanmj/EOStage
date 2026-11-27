//! Renaming a conflicting file aside (downloader D3). The Owner's rule: a
//! file that cannot be reused is neither deleted nor overwritten; it is kept
//! in its directory as "<name>.eostage-conflict-<time>" for the user to
//! remove, and the planned file is downloaded anew.

use std::io;
use std::path::{Path, PathBuf};

/// The marker between the original name and the conflict time.
pub(crate) const CONFLICT_MARKER: &str = ".eostage-conflict-";

const MAX_NAME_ATTEMPTS: u32 = 99;

/// Why a conflicting file could not be renamed aside.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum QuarantineError {
    /// Every conflict name for this time already exists.
    NamesExhausted,
    /// The rename failed (in use, access denied, missing parent).
    Io(io::ErrorKind),
}

/// The conflict name of `file_name` for `stamp` (a compact UTC time such as
/// 20261020T010203Z); the second and later attempts add "-2", "-3", ...
pub(crate) fn conflict_name(file_name: &str, stamp: &str, attempt: u32) -> String {
    if attempt <= 1 {
        format!("{file_name}{CONFLICT_MARKER}{stamp}")
    } else {
        format!("{file_name}{CONFLICT_MARKER}{stamp}-{attempt}")
    }
}

/// The compact form of an RFC 3339 UTC time ("2026-10-20T01:02:03Z" becomes
/// "20261020T010203Z"), safe in Windows file names.
pub(crate) fn conflict_stamp(rfc3339_utc: &str) -> String {
    rfc3339_utc
        .chars()
        .filter(|character| !matches!(character, '-' | ':'))
        .collect()
}

/// Renames `destination` to its first free conflict name in the same
/// directory, never replacing an existing file, and returns the new path.
pub(crate) fn quarantine_conflict(
    destination: &Path,
    stamp: &str,
) -> Result<PathBuf, QuarantineError> {
    let parent = destination
        .parent()
        .ok_or(QuarantineError::Io(io::ErrorKind::InvalidInput))?;
    let file_name = destination
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or(QuarantineError::Io(io::ErrorKind::InvalidInput))?;
    for attempt in 1..=MAX_NAME_ATTEMPTS {
        let target = parent.join(conflict_name(file_name, stamp, attempt));
        match rename_no_replace(destination, &target) {
            Ok(()) => return Ok(target),
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => continue,
            Err(error) => return Err(QuarantineError::Io(error.kind())),
        }
    }
    Err(QuarantineError::NamesExhausted)
}

#[cfg(windows)]
fn rename_no_replace(from: &Path, to: &Path) -> io::Result<()> {
    use std::os::windows::ffi::OsStrExt;
    use windows_sys::Win32::Storage::FileSystem::MoveFileExW;

    let wide = |path: &Path| -> Vec<u16> {
        crate::long_path::native_path(path)
            .as_os_str()
            .encode_wide()
            .chain(Some(0))
            .collect()
    };
    let (source, target) = (wide(from), wide(to));
    // Flags 0: never MOVEFILE_REPLACE_EXISTING, never a copy to another volume.
    // SAFETY: both buffers are NUL-terminated and outlive the call.
    if unsafe { MoveFileExW(source.as_ptr(), target.as_ptr(), 0) } == 0 {
        let error = io::Error::last_os_error();
        // ERROR_FILE_EXISTS, ERROR_ALREADY_EXISTS
        if matches!(error.raw_os_error(), Some(80 | 183)) {
            return Err(io::Error::new(io::ErrorKind::AlreadyExists, error));
        }
        return Err(error);
    }
    Ok(())
}

#[cfg(not(windows))]
fn rename_no_replace(from: &Path, to: &Path) -> io::Result<()> {
    // A hard link fails when the target exists; the original name is removed
    // only after the conflict name holds the file.
    std::fs::hard_link(from, to)?;
    std::fs::remove_file(from)
}
