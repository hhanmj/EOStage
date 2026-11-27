//! The identity of a regular file on its volume (downloader D3, local-first
//! dedup). Safe promotion compares source identities with it, and the local
//! reuse ledger records it so that a file whose identity is unchanged since it
//! was verified can be reused without reading its content again.

use std::fs::File;

/// Whether the volume keeps file identities stable across renames and time.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum IdentityStrength {
    /// NTFS or ReFS on Windows, a local Unix file system: volume serial, file
    /// index and change time identify the file and its content generation.
    Stable,
    /// Other volumes (FAT, exFAT, network shares): the identity may be reused
    /// or may not change with content; never enough to skip reading.
    #[cfg_attr(not(windows), allow(dead_code))]
    Weak,
}

/// The identity and status of an open regular file.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct FileStatus {
    pub(crate) platform: &'static str,
    pub(crate) volume_id: u64,
    pub(crate) file_id: u64,
    pub(crate) size: u64,
    pub(crate) links: u64,
    /// Last write time, in 100 ns units on Windows and nanoseconds on Unix.
    pub(crate) modified: Option<i128>,
    /// Last metadata change time, same units; None when the volume has none.
    pub(crate) changed: Option<i128>,
    pub(crate) strength: IdentityStrength,
}

/// Why an open file has no usable identity.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum FileStatusError {
    /// A directory or a reparse point (link, junction, placeholder).
    NotRegular,
    /// The operating system did not report the identity.
    Unavailable,
}

#[cfg(windows)]
pub(crate) fn file_status(file: &File) -> Result<FileStatus, FileStatusError> {
    use std::os::windows::io::AsRawHandle;
    use windows_sys::Win32::Storage::FileSystem::{
        FileBasicInfo, GetFileInformationByHandle, GetFileInformationByHandleEx,
        GetVolumeInformationByHandleW, BY_HANDLE_FILE_INFORMATION, FILE_BASIC_INFO,
    };

    let handle = file.as_raw_handle() as _;
    let mut info = unsafe { std::mem::zeroed::<BY_HANDLE_FILE_INFORMATION>() };
    // SAFETY: `handle` is the open file's handle and `info` outlives the call.
    if unsafe { GetFileInformationByHandle(handle, &mut info) } == 0 || info.nNumberOfLinks == 0 {
        return Err(FileStatusError::Unavailable);
    }
    // FILE_ATTRIBUTE_DIRECTORY | FILE_ATTRIBUTE_REPARSE_POINT
    if info.dwFileAttributes & (0x0000_0010 | 0x0000_0400) != 0 {
        return Err(FileStatusError::NotRegular);
    }
    let mut basic = unsafe { std::mem::zeroed::<FILE_BASIC_INFO>() };
    // SAFETY: `basic` is a FILE_BASIC_INFO buffer of the size passed.
    let changed = (unsafe {
        GetFileInformationByHandleEx(
            handle,
            FileBasicInfo,
            (&mut basic as *mut FILE_BASIC_INFO).cast(),
            std::mem::size_of::<FILE_BASIC_INFO>() as u32,
        )
    } != 0)
        .then_some(i128::from(basic.ChangeTime));
    let mut file_system = [0_u16; 32];
    // SAFETY: the file-system name buffer outlives the call and its length is
    // passed; the other outputs are optional.
    let named = unsafe {
        GetVolumeInformationByHandleW(
            handle,
            std::ptr::null_mut(),
            0,
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            file_system.as_mut_ptr(),
            file_system.len() as u32,
        )
    } != 0;
    let name_length = file_system.iter().position(|unit| *unit == 0).unwrap_or(0);
    let file_system = String::from_utf16_lossy(&file_system[..name_length]);
    let strength = if named
        && changed.is_some()
        && (file_system.eq_ignore_ascii_case("NTFS") || file_system.eq_ignore_ascii_case("ReFS"))
    {
        IdentityStrength::Stable
    } else {
        IdentityStrength::Weak
    };
    let modified = (i128::from(info.ftLastWriteTime.dwHighDateTime) << 32)
        | i128::from(info.ftLastWriteTime.dwLowDateTime);
    Ok(FileStatus {
        platform: "windows",
        volume_id: u64::from(info.dwVolumeSerialNumber),
        file_id: (u64::from(info.nFileIndexHigh) << 32) | u64::from(info.nFileIndexLow),
        size: (u64::from(info.nFileSizeHigh) << 32) | u64::from(info.nFileSizeLow),
        links: u64::from(info.nNumberOfLinks),
        modified: Some(modified),
        changed,
        strength,
    })
}

#[cfg(unix)]
pub(crate) fn file_status(file: &File) -> Result<FileStatus, FileStatusError> {
    use std::os::unix::fs::MetadataExt;

    let metadata = file.metadata().map_err(|_| FileStatusError::Unavailable)?;
    if !metadata.is_file() {
        return Err(FileStatusError::NotRegular);
    }
    if metadata.nlink() == 0 {
        return Err(FileStatusError::Unavailable);
    }
    Ok(FileStatus {
        platform: "unix",
        volume_id: metadata.dev(),
        file_id: metadata.ino(),
        size: metadata.len(),
        links: metadata.nlink(),
        modified: Some(
            i128::from(metadata.mtime()) * 1_000_000_000 + i128::from(metadata.mtime_nsec()),
        ),
        changed: Some(
            i128::from(metadata.ctime()) * 1_000_000_000 + i128::from(metadata.ctime_nsec()),
        ),
        strength: IdentityStrength::Stable,
    })
}

#[cfg(all(not(windows), not(unix)))]
pub(crate) fn file_status(_file: &File) -> Result<FileStatus, FileStatusError> {
    Err(FileStatusError::Unavailable)
}

#[cfg(test)]
mod tests {
    use super::{file_status, IdentityStrength};

    #[test]
    fn an_open_regular_file_reports_a_stable_identity_that_follows_its_content() {
        let root = std::env::temp_dir().join(format!("insar-file-identity-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();
        let path = root.join("product.zip");
        std::fs::write(&path, b"abc").unwrap();
        let first = file_status(&std::fs::File::open(&path).unwrap()).unwrap();
        assert_eq!(first.size, 3);
        assert_eq!(first.links, 1);
        assert!(first.modified.is_some());
        let again = file_status(&std::fs::File::open(&path).unwrap()).unwrap();
        assert_eq!(first, again, "nothing changed");
        // Renaming keeps the identity on the same volume.
        let renamed = root.join("renamed.zip");
        std::fs::rename(&path, &renamed).unwrap();
        let moved = file_status(&std::fs::File::open(&renamed).unwrap()).unwrap();
        assert_eq!(
            (moved.volume_id, moved.file_id),
            (first.volume_id, first.file_id)
        );
        if first.strength == IdentityStrength::Stable {
            assert!(first.changed.is_some());
        }
        let _ = std::fs::remove_dir_all(&root);
    }
}
