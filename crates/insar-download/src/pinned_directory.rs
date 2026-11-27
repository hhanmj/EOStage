use std::fmt;
use std::fs::{self, File, OpenOptions};
use std::io;
use std::path::{Path, PathBuf};

#[derive(Clone, Debug, PartialEq, Eq)]
struct DirectoryIdentity {
    platform: &'static str,
    device_id: u64,
    file_id: u64,
}

/// Canonical absolute application-owned directory with a retained identity
/// handle. Windows opens deny delete sharing, so the root cannot be renamed or
/// replaced while the workflow owns it. Unix retains the directory inode and
/// every path-based operation rechecks the canonical path against that inode.
pub(crate) struct PinnedDirectory {
    path: PathBuf,
    handle: File,
    identity: DirectoryIdentity,
}

/// Exact child publication authority bound to a cloned retained directory handle.
/// The child is one already-validated component, never a parsed pathname.
#[cfg(any(feature = "nisar-subset-production", test))]
pub(crate) struct PinnedDirectoryPublication {
    path: PathBuf,
    handle: File,
    identity: DirectoryIdentity,
    component: String,
}

#[cfg(any(feature = "nisar-subset-production", test))]
impl PinnedDirectoryPublication {
    pub(crate) fn component(&self) -> &str {
        &self.component
    }

    pub(crate) fn exact_child_path(&self) -> io::Result<PathBuf> {
        self.verify()?;
        Ok(self.path.join(&self.component))
    }

    pub(crate) fn try_clone(&self) -> io::Result<Self> {
        self.verify()?;
        let handle = self.handle.try_clone()?;
        if directory_identity(&handle)? != self.identity {
            return Err(io::Error::other(
                "cloned publication capability changed identity",
            ));
        }
        Ok(Self {
            path: self.path.clone(),
            handle,
            identity: self.identity.clone(),
            component: self.component.clone(),
        })
    }

    #[cfg(test)]
    pub(crate) fn directory_identity_marker(&self) -> String {
        format!(
            "{}:{}:{}",
            self.identity.platform, self.identity.device_id, self.identity.file_id
        )
    }

    pub(crate) fn verify(&self) -> io::Result<()> {
        if !safe_component(&self.component) || directory_identity(&self.handle)? != self.identity {
            return Err(io::Error::other(
                "pinned publication capability changed identity",
            ));
        }
        let metadata = fs::symlink_metadata(&self.path)?;
        if metadata.file_type().is_symlink() || !metadata.is_dir() {
            return Err(io::Error::other(
                "pinned publication directory path was replaced",
            ));
        }
        let reopened = open_directory_nofollow(&self.path)?;
        if directory_identity(&reopened)? != self.identity {
            return Err(io::Error::other(
                "pinned publication directory path changed identity",
            ));
        }
        Ok(())
    }

    #[cfg(any(target_os = "linux", target_os = "android", target_os = "macos"))]
    #[cfg_attr(target_os = "macos", allow(dead_code))]
    pub(crate) fn try_clone_directory_handle(&self) -> io::Result<File> {
        self.verify()?;
        let handle = self.handle.try_clone()?;
        if directory_identity(&handle)? != self.identity {
            return Err(io::Error::other(
                "cloned publication directory changed identity",
            ));
        }
        Ok(handle)
    }
}

impl PinnedDirectory {
    pub(crate) fn open_existing(path: &Path) -> io::Result<Self> {
        if path.as_os_str().is_empty() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "directory path is empty",
            ));
        }
        let metadata = fs::symlink_metadata(path)?;
        if metadata.file_type().is_symlink() || !metadata.is_dir() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "directory is not real",
            ));
        }
        let path = fs::canonicalize(path)?;
        let handle = open_directory_nofollow(&path)?;
        let identity = directory_identity(&handle)?;
        stable_handle_root(&path, &handle)?;
        let pinned = Self {
            path,
            handle,
            identity,
        };
        pinned.verify()?;
        Ok(pinned)
    }

    pub(crate) fn create(path: &Path) -> io::Result<Self> {
        if path.as_os_str().is_empty() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "directory path is empty",
            ));
        }
        let path = if path.is_absolute() {
            path.to_path_buf()
        } else {
            std::env::current_dir()?.join(path)
        };
        fs::create_dir_all(&path)?;
        let path = fs::canonicalize(path)?;
        if !path.is_absolute() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "directory path is not absolute",
            ));
        }
        let metadata = fs::symlink_metadata(&path)?;
        if metadata.file_type().is_symlink() || !metadata.is_dir() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "directory path is not a real directory",
            ));
        }
        let handle = open_directory_nofollow(&path)?;
        let identity = directory_identity(&handle)?;
        stable_handle_root(&path, &handle)?;
        let pinned = Self {
            path,
            handle,
            identity,
        };
        pinned.verify()?;
        Ok(pinned)
    }

    pub(crate) fn path(&self) -> &Path {
        &self.path
    }

    pub(crate) fn identity_marker(&self) -> String {
        format!(
            "{}:{}:{}",
            self.identity.platform, self.identity.device_id, self.identity.file_id
        )
    }

    #[cfg(any(feature = "nisar-subset-production", test))]
    pub(crate) fn publication_capability(
        &self,
        component: &str,
    ) -> io::Result<PinnedDirectoryPublication> {
        self.verify()?;
        if !safe_component(component) {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "unsafe publication child component",
            ));
        }
        let handle = self.handle.try_clone()?;
        if directory_identity(&handle)? != self.identity {
            return Err(io::Error::other(
                "publication directory clone changed identity",
            ));
        }
        Ok(PinnedDirectoryPublication {
            path: self.path.clone(),
            handle,
            identity: self.identity.clone(),
            component: component.to_owned(),
        })
    }

    pub(crate) fn verify(&self) -> io::Result<()> {
        if directory_identity(&self.handle)? != self.identity {
            return Err(io::Error::other("pinned directory handle changed identity"));
        }
        let metadata = fs::symlink_metadata(&self.path)?;
        if metadata.file_type().is_symlink() || !metadata.is_dir() {
            return Err(io::Error::other("pinned directory path was replaced"));
        }
        let reopened = open_directory_nofollow(&self.path)?;
        if directory_identity(&reopened)? != self.identity {
            return Err(io::Error::other("pinned directory path changed identity"));
        }
        Ok(())
    }

    pub(crate) fn join_component(&self, name: &str) -> io::Result<PathBuf> {
        self.verify()?;
        self.stable_component(name)
    }

    pub(crate) fn stable_component(&self, name: &str) -> io::Result<PathBuf> {
        if !safe_component(name) {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "unsafe child path component",
            ));
        }
        Ok(stable_handle_root(&self.path, &self.handle)?.join(name))
    }

    pub(crate) fn canonical_component(&self, name: &str) -> io::Result<PathBuf> {
        if !safe_component(name) {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "unsafe child path component",
            ));
        }
        Ok(self.path.join(name))
    }

    /// The volume's file system name and whether the volume is local (fixed
    /// or removable), observed through the retained handle. Workspace
    /// rebinding accepts only a local NTFS volume (design 2026-10-17 §3).
    #[cfg(all(windows, feature = "nisar-subset-production"))]
    pub(crate) fn volume_facts(&self) -> io::Result<(String, bool)> {
        use std::os::windows::io::AsRawHandle;
        use windows_sys::Win32::Storage::FileSystem::{
            GetDriveTypeW, GetVolumeInformationByHandleW,
        };

        self.verify()?;
        let mut file_system = [0_u16; 64];
        // SAFETY: the handle is a live directory handle owned by self and the
        // output buffer length matches the buffer.
        let succeeded = unsafe {
            GetVolumeInformationByHandleW(
                self.handle.as_raw_handle() as _,
                std::ptr::null_mut(),
                0,
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                file_system.as_mut_ptr(),
                file_system.len() as u32,
            )
        };
        if succeeded == 0 {
            return Err(io::Error::last_os_error());
        }
        let length = file_system
            .iter()
            .position(|unit| *unit == 0)
            .unwrap_or(file_system.len());
        let file_system = String::from_utf16_lossy(&file_system[..length]);
        // Only a verbatim drive path (\\?\X:\...) can be local; UNC and device
        // paths are not. Review 2026-10-17 round 3, P2-4: the drive type belongs
        // to the volume that holds the directory, which for a volume mounted on
        // an NTFS folder is not the volume of the drive letter. Ask for the type
        // of the volume mount point of the canonical path (the drive root or
        // the mount folder) instead of the drive root.
        let text = self.path.as_os_str().to_string_lossy();
        let bytes = text.as_bytes();
        let local = if bytes.len() >= 7
            && bytes.starts_with(br"\\?\")
            && bytes[4].is_ascii_alphabetic()
            && bytes[5] == b':'
            && bytes[6] == b'\\'
        {
            let root = volume_mount_point(&self.path)?;
            // SAFETY: `root` is a NUL-terminated UTF-16 volume mount point.
            let kind = unsafe { GetDriveTypeW(root.as_ptr()) };
            // DRIVE_REMOVABLE (2) or DRIVE_FIXED (3).
            matches!(kind, 2 | 3)
        } else {
            false
        };
        Ok((file_system, local))
    }

    #[cfg(all(not(windows), feature = "nisar-subset-production"))]
    pub(crate) fn volume_facts(&self) -> io::Result<(String, bool)> {
        Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "volume facts are observed on Windows only",
        ))
    }

    /// Remove this exact, identity-verified directory after the caller has
    /// completed all work that needs the retained handle. The handle is
    /// released only after verification so Windows can remove the directory;
    /// callers must keep the target task-scoped and must not use this for a
    /// parent/output root.
    pub(crate) fn remove_after_verify(self) -> io::Result<()> {
        self.verify()?;
        let path = self.path.clone();
        let handle = self.handle;
        drop(handle);
        fs::remove_dir_all(path)
    }
}

/// The volume mount point that contains `path` (Review 2026-10-17 round 3,
/// P2-4): the drive root, or the NTFS folder a volume is mounted on, with its
/// trailing separator, as a NUL-terminated UTF-16 string for the volume APIs.
#[cfg(all(windows, feature = "nisar-subset-production"))]
fn volume_mount_point(path: &Path) -> io::Result<Vec<u16>> {
    use std::os::windows::ffi::OsStrExt;
    use windows_sys::Win32::Storage::FileSystem::GetVolumePathNameW;

    let wide = path
        .as_os_str()
        .encode_wide()
        .chain(std::iter::once(0))
        .collect::<Vec<u16>>();
    // The mount point is a prefix of the path plus at most a trailing
    // separator, so the path's own length and two units always suffice.
    let mut root = vec![0_u16; wide.len().saturating_add(2).max(261)];
    let capacity = u32::try_from(root.len())
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "volume path is too long"))?;
    // SAFETY: `wide` is NUL-terminated and `root` holds `capacity` units.
    if unsafe { GetVolumePathNameW(wide.as_ptr(), root.as_mut_ptr(), capacity) } == 0 {
        return Err(io::Error::last_os_error());
    }
    let length = root
        .iter()
        .position(|unit| *unit == 0)
        .ok_or_else(|| io::Error::other("volume mount point is not terminated"))?;
    root.truncate(length + 1);
    Ok(root)
}

impl fmt::Debug for PinnedDirectory {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("PinnedDirectory")
            .field("path", &self.path)
            .field("identity", &self.identity)
            .finish_non_exhaustive()
    }
}

impl PartialEq for PinnedDirectory {
    fn eq(&self, other: &Self) -> bool {
        self.path == other.path && self.identity == other.identity
    }
}

impl Eq for PinnedDirectory {}

pub(crate) fn safe_component(value: &str) -> bool {
    !value.is_empty()
        && value != "."
        && value != ".."
        && value.len() <= 255
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-'))
}

#[cfg(windows)]
fn open_directory_nofollow(path: &Path) -> io::Result<File> {
    use std::os::windows::fs::OpenOptionsExt;

    OpenOptions::new()
        .read(true)
        // Peers may enumerate/read/write children, but cannot rename or replace
        // the root while this identity handle is retained.
        .share_mode(0x0000_0001 | 0x0000_0002)
        .custom_flags(0x0200_0000 | 0x0020_0000)
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
fn open_directory_nofollow(path: &Path) -> io::Result<File> {
    use std::os::unix::fs::OpenOptionsExt;

    OpenOptions::new()
        .read(true)
        .custom_flags(unix_o_nofollow())
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
fn open_directory_nofollow(_path: &Path) -> io::Result<File> {
    Err(io::Error::new(
        io::ErrorKind::Unsupported,
        "pinned directories are unsupported on this Unix target",
    ))
}

#[cfg(all(not(windows), not(unix)))]
fn open_directory_nofollow(_path: &Path) -> io::Result<File> {
    Err(io::Error::new(
        io::ErrorKind::Unsupported,
        "pinned directories are unsupported on this platform",
    ))
}

#[cfg(windows)]
fn directory_identity(file: &File) -> io::Result<DirectoryIdentity> {
    use std::os::windows::io::AsRawHandle;
    use windows_sys::Win32::Storage::FileSystem::{
        GetFileInformationByHandle, BY_HANDLE_FILE_INFORMATION,
    };

    let mut info = unsafe { std::mem::zeroed::<BY_HANDLE_FILE_INFORMATION>() };
    let succeeded = unsafe { GetFileInformationByHandle(file.as_raw_handle() as _, &mut info) };
    if succeeded == 0
        || info.dwFileAttributes & 0x0000_0400 != 0
        || info.dwFileAttributes & 0x0000_0010 == 0
    {
        return Err(io::Error::other("unsafe pinned directory handle"));
    }
    Ok(DirectoryIdentity {
        platform: "windows",
        device_id: u64::from(info.dwVolumeSerialNumber),
        file_id: (u64::from(info.nFileIndexHigh) << 32) | u64::from(info.nFileIndexLow),
    })
}

#[cfg(unix)]
fn directory_identity(file: &File) -> io::Result<DirectoryIdentity> {
    use std::os::unix::fs::MetadataExt;

    let metadata = file.metadata()?;
    if !metadata.is_dir() {
        return Err(io::Error::other("unsafe pinned directory handle"));
    }
    Ok(DirectoryIdentity {
        platform: "unix",
        device_id: metadata.dev(),
        file_id: metadata.ino(),
    })
}

#[cfg(all(not(windows), not(unix)))]
fn directory_identity(_file: &File) -> io::Result<DirectoryIdentity> {
    Err(io::Error::new(
        io::ErrorKind::Unsupported,
        "directory identity is unsupported on this platform",
    ))
}

#[cfg(windows)]
fn stable_handle_root(path: &Path, _handle: &File) -> io::Result<PathBuf> {
    Ok(path.to_path_buf())
}

#[cfg(any(
    target_os = "linux",
    target_os = "android",
    target_os = "solaris",
    target_os = "illumos"
))]
fn stable_handle_root(_path: &Path, handle: &File) -> io::Result<PathBuf> {
    use std::os::unix::io::AsRawFd;

    let root = PathBuf::from(format!("/proc/self/fd/{}", handle.as_raw_fd()));
    if fs::metadata(&root)?.is_dir() {
        Ok(root)
    } else {
        Err(io::Error::other("pinned directory fd path is unavailable"))
    }
}

#[cfg(any(
    target_os = "macos",
    target_os = "ios",
    target_os = "freebsd",
    target_os = "dragonfly",
    target_os = "netbsd",
    target_os = "openbsd"
))]
fn stable_handle_root(path: &Path, handle: &File) -> io::Result<PathBuf> {
    if handle.metadata()?.is_dir() {
        Ok(path.to_path_buf())
    } else {
        Err(io::Error::other(
            "pinned directory handle is not a directory",
        ))
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
fn stable_handle_root(_path: &Path, _handle: &File) -> io::Result<PathBuf> {
    Err(io::Error::new(
        io::ErrorKind::Unsupported,
        "stable directory fd paths are unsupported on this Unix target",
    ))
}

#[cfg(all(not(windows), not(unix)))]
fn stable_handle_root(_path: &Path, _handle: &File) -> io::Result<PathBuf> {
    Err(io::Error::new(
        io::ErrorKind::Unsupported,
        "stable directory handles are unsupported on this platform",
    ))
}

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
const fn unix_o_nofollow() -> i32 {
    0x0000_8000
}

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
const fn unix_o_nofollow() -> i32 {
    0x0002_0000
}

#[cfg(all(
    target_os = "android",
    any(target_arch = "arm", target_arch = "aarch64")
))]
const fn unix_o_nofollow() -> i32 {
    0x0000_8000
}

#[cfg(all(
    target_os = "android",
    any(target_arch = "x86", target_arch = "x86_64")
))]
const fn unix_o_nofollow() -> i32 {
    0x0002_0000
}

#[cfg(all(target_os = "android", target_arch = "riscv64"))]
const fn unix_o_nofollow() -> i32 {
    0x0040_0000
}

#[cfg(any(target_os = "solaris", target_os = "illumos"))]
const fn unix_o_nofollow() -> i32 {
    0x0002_0000
}

#[cfg(any(
    target_os = "macos",
    target_os = "ios",
    target_os = "freebsd",
    target_os = "dragonfly",
    target_os = "netbsd",
    target_os = "openbsd"
))]
const fn unix_o_nofollow() -> i32 {
    0x0000_0100
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU64, Ordering};

    static NEXT_ID: AtomicU64 = AtomicU64::new(0);

    #[test]
    fn archived_recovery_open_existing_never_creates_missing_directory() {
        let id = NEXT_ID.fetch_add(1, Ordering::Relaxed);
        let base =
            std::env::temp_dir().join(format!("insar-archive-open-{}-{id}", std::process::id()));
        let missing = base.join("missing");
        assert!(PinnedDirectory::open_existing(&missing).is_err());
        assert!(!base.exists());
        fs::create_dir_all(&base).unwrap();
        let pinned =
            PinnedDirectory::open_existing(&base).expect("existing directory can be read-pinned");
        pinned.verify().unwrap();
        drop(pinned);
        fs::remove_dir(&base).unwrap();
    }

    #[test]
    fn canonical_root_is_absolute_and_rejects_unsafe_components() {
        let id = NEXT_ID.fetch_add(1, Ordering::Relaxed);
        let base = std::env::temp_dir().join(format!(
            "insar-pinned-directory-{}-{id}",
            std::process::id()
        ));
        let root = base.join("root");
        let pinned = PinnedDirectory::create(&root).unwrap();
        assert!(pinned.path().is_absolute());
        let child = pinned.join_component("safe-child.json").unwrap();
        fs::write(&child, b"{}").unwrap();
        assert_eq!(fs::read(&child).unwrap(), b"{}");
        for unsafe_name in ["", ".", "..", "../escape", "..\\escape", "/absolute"] {
            assert!(!safe_component(unsafe_name));
            assert!(pinned.join_component(unsafe_name).is_err());
        }
        drop(pinned);
        let _ = fs::remove_dir_all(base);
    }

    #[test]
    fn publication_capability_binds_retained_root_and_exact_component() {
        let id = NEXT_ID.fetch_add(1, Ordering::Relaxed);
        let base = std::env::temp_dir().join(format!(
            "insar-pinned-publication-{}-{id}",
            std::process::id()
        ));
        let root = base.join("root");
        let pinned = PinnedDirectory::create(&root).unwrap();
        let publication = pinned.publication_capability("artifact.h5").unwrap();
        assert_eq!(publication.component(), "artifact.h5");
        assert_eq!(
            publication.directory_identity_marker(),
            pinned.identity_marker()
        );
        for unsafe_name in ["", ".", "..", "../escape", "..\\escape", "/absolute"] {
            assert!(pinned.publication_capability(unsafe_name).is_err());
        }
        drop(publication);
        drop(pinned);
        let _ = fs::remove_dir_all(base);
    }

    #[cfg(windows)]
    #[test]
    fn retained_windows_root_handle_denies_directory_replacement() {
        let id = NEXT_ID.fetch_add(1, Ordering::Relaxed);
        let base = std::env::temp_dir().join(format!(
            "insar-pinned-directory-windows-{}-{id}",
            std::process::id()
        ));
        let root = base.join("root");
        let displaced = base.join("displaced");
        let pinned = PinnedDirectory::create(&root).unwrap();
        assert!(fs::rename(&root, &displaced).is_err());
        pinned.verify().unwrap();
        drop(pinned);
        let _ = fs::remove_dir_all(base);
    }

    /// Review 2026-10-17 round 3, P2-4: the drive type is read for the volume
    /// mount point that holds the directory, not for its drive letter. A volume
    /// mounted on a folder needs administrator rights and a spare volume to
    /// create, so this pins the derivation on a real temporary directory: the
    /// mount point is the verbatim volume prefix of the canonical path with a
    /// trailing separator, and a directory on a fixed local disk is local.
    #[cfg(all(windows, feature = "nisar-subset-production"))]
    #[test]
    fn volume_facts_read_the_drive_type_of_the_volume_mount_point() {
        let id = NEXT_ID.fetch_add(1, Ordering::Relaxed);
        let base =
            std::env::temp_dir().join(format!("insar-pinned-volume-{}-{id}", std::process::id()));
        let pinned = PinnedDirectory::create(&base.join("root")).unwrap();
        let mount = volume_mount_point(pinned.path()).unwrap();
        assert_eq!(mount.last(), Some(&0));
        let mount = String::from_utf16(&mount[..mount.len() - 1]).unwrap();
        let path = pinned.path().to_string_lossy().into_owned();
        assert!(
            mount.starts_with(r"\\?\") && mount.ends_with('\\'),
            "{mount}"
        );
        assert!(
            path.to_ascii_lowercase()
                .starts_with(&mount.to_ascii_lowercase()),
            "{mount} is not the volume prefix of {path}"
        );
        let (file_system, local) = pinned.volume_facts().unwrap();
        assert!(!file_system.is_empty());
        assert!(local, "{mount} ({file_system})");
        drop(pinned);
        let _ = fs::remove_dir_all(base);
    }

    #[cfg(unix)]
    #[test]
    fn unix_root_identity_recheck_detects_path_replacement() {
        use std::os::unix::fs::symlink;

        let id = NEXT_ID.fetch_add(1, Ordering::Relaxed);
        let base = std::env::temp_dir().join(format!(
            "insar-pinned-directory-unix-{}-{id}",
            std::process::id()
        ));
        let root = base.join("root");
        let displaced = base.join("displaced");
        let outside = base.join("outside");
        fs::create_dir_all(&outside).unwrap();
        let pinned = PinnedDirectory::create(&root).unwrap();
        fs::rename(&root, &displaced).unwrap();
        symlink(&outside, &root).unwrap();
        assert!(pinned.verify().is_err());
        drop(pinned);
        let _ = fs::remove_file(&root);
        let _ = fs::remove_dir_all(base);
    }
}
