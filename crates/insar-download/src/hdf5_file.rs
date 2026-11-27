//! Opening HDF5 files by path, including the descriptor paths of unnamed
//! files.
//!
//! On Linux/Android the NISAR writer creates, seals and reads unnamed
//! candidates through `/proc/self/fd/<n>`. HDF5's default sec2 driver
//! resolves a symlinked file name with `realpath()`, which fails for an
//! unnamed file ("can't retrieve real path for file"). The stdio driver has
//! the same allocation features (so the same file layout) but no POSIX
//! compatible handle, so HDF5 skips that resolution; descriptor paths use it.
//! Every other path keeps HDF5's default driver.

use std::path::Path;

use hdf5::plist::file_access::FileAccessBuilder;
use hdf5::{File, FileBuilder};

/// Whether `path` names an open file of this process (`/proc/self/fd/<n>` or `/dev/fd/<n>`).
fn is_descriptor_path(path: &Path) -> bool {
    (cfg!(any(target_os = "linux", target_os = "android"))
        && path
            .to_str()
            .and_then(|text| text.strip_prefix("/proc/self/fd/"))
            .is_some_and(|descriptor| {
                !descriptor.is_empty() && descriptor.bytes().all(|byte| byte.is_ascii_digit())
            }))
        || (cfg!(target_os = "macos")
            && path
                .to_str()
                .and_then(|text| text.strip_prefix("/dev/fd/"))
                .is_some_and(|descriptor| {
                    !descriptor.is_empty() && descriptor.bytes().all(|byte| byte.is_ascii_digit())
                }))
}

/// Selects the file driver for `path` on an existing file-access builder.
pub(crate) fn driver_for<'a>(
    access: &'a mut FileAccessBuilder,
    path: &Path,
) -> &'a mut FileAccessBuilder {
    if is_descriptor_path(path) {
        #[cfg(unix)]
        access.file_locking(false);
        access.stdio()
    } else {
        access
    }
}

/// A file builder whose driver suits `path`.
pub(crate) fn builder(path: &Path) -> FileBuilder {
    let mut builder = File::with_options();
    builder.with_fapl(|access| driver_for(access, path));
    builder
}

/// `hdf5::File::open` for `path` (read-only).
pub(crate) fn open(path: impl AsRef<Path>) -> hdf5::Result<File> {
    let path = crate::long_path::native_path(path.as_ref());
    builder(&path).open(&path)
}

/// `hdf5::File::open_rw` for `path`.
// Only a Linux test-only snapshot swap uses it today.
#[cfg_attr(
    not(all(
        test,
        any(target_os = "linux", target_os = "android", target_os = "macos")
    )),
    allow(dead_code)
)]
pub(crate) fn open_rw(path: impl AsRef<Path>) -> hdf5::Result<File> {
    let path = crate::long_path::native_path(path.as_ref());
    builder(&path).open_rw(&path)
}

#[cfg(test)]
mod tests {
    use super::is_descriptor_path;
    use std::path::Path;

    #[test]
    fn only_numeric_proc_self_descriptors_are_descriptor_paths() {
        let linux = cfg!(any(target_os = "linux", target_os = "android"));
        assert_eq!(is_descriptor_path(Path::new("/proc/self/fd/12")), linux);
        for other in [
            "/proc/self/fd/",
            "/proc/self/fd/12/x",
            "/proc/self/fd/1a",
            "/proc/1234/fd/12",
            "/data/out.h5",
            "relative.h5",
        ] {
            assert!(!is_descriptor_path(Path::new(other)), "{other}");
        }
    }
}
