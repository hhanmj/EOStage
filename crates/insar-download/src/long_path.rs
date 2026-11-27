//! Paths handed to code that does not add the Windows extended-length prefix
//! itself: the HDF5 C library and Windows API calls that take a raw path.
//!
//! L3 gate (2026-10-19): `std::fs` adds the `\\?\` prefix to a long path
//! itself; the HDF5 C library and raw-path Windows calls do not, so a path of
//! MAX_PATH (260) characters or more failed there. A NISAR product under a deep
//! output directory passed every byte check, could not be opened by HDF5 and was
//! refused as NISAR_HDF5_STRUCTURE_INVALID; its promotion (a rename by handle to
//! the raw destination path) failed in the same way.

use std::path::{Path, PathBuf};

/// Paths of at least this many UTF-16 units are handed over in their
/// extended-length form on Windows; shorter paths are unchanged. The margin
/// below MAX_PATH (260) leaves room for the terminator and derived names.
#[cfg(windows)]
const EXTENDED_LENGTH_FROM_UNITS: usize = 240;

/// The path to hand to the HDF5 C library or a raw-path Windows call for an
/// existing or new entry of an existing directory. On Windows a long path
/// becomes the canonical extended-length (`\\?\`) form of its directory
/// joined with the unchanged final component: the final component is never
/// resolved, so a rename still replaces that directory entry and an open still
/// opens what the original path names. Every other path, and a long path whose
/// directory cannot be resolved, is returned unchanged.
pub(crate) fn native_path(path: impl AsRef<Path>) -> PathBuf {
    let path = path.as_ref();
    #[cfg(windows)]
    {
        use std::os::windows::ffi::OsStrExt;
        if path.as_os_str().encode_wide().count() >= EXTENDED_LENGTH_FROM_UNITS {
            if let Some(extended) = extended_length(path) {
                return extended;
            }
        }
    }
    path.to_path_buf()
}

/// `fs::canonicalize` returns the extended-length form on Windows.
#[cfg(windows)]
fn extended_length(path: &Path) -> Option<PathBuf> {
    if let Some(std::path::Component::Prefix(prefix)) = path.components().next() {
        if prefix.kind().is_verbatim() {
            return None;
        }
    }
    let name = path.file_name()?;
    let directory = std::fs::canonicalize(path.parent()?).ok()?;
    Some(directory.join(name))
}

#[cfg(all(test, windows))]
mod tests {
    use super::native_path;
    use std::path::Path;

    #[test]
    fn a_short_path_is_handed_over_unchanged() {
        let path = Path::new(r"C:\data\NISAR.h5");
        assert_eq!(native_path(path), path);
    }

    #[test]
    fn a_long_path_is_handed_over_in_the_extended_length_form_of_the_same_entry() {
        let root =
            std::env::temp_dir().join(format!("insar-native-long-path-{}", std::process::id()));
        let mut directory = root.clone();
        while directory.as_os_str().len() < 250 {
            directory.push("p".repeat(40));
        }
        std::fs::create_dir_all(&directory).unwrap();
        let existing = directory.join("existing.h5");
        std::fs::write(&existing, b"existing").unwrap();

        let opened = native_path(&existing);
        assert!(
            opened.to_string_lossy().starts_with(r"\\?\"),
            "{}",
            opened.display()
        );
        assert!(opened.ends_with("existing.h5"));
        assert_eq!(std::fs::read(&opened).unwrap(), b"existing");
        let created = native_path(directory.join("new.h5"));
        assert!(
            created.to_string_lossy().starts_with(r"\\?\"),
            "{}",
            created.display()
        );
        assert!(created.ends_with("new.h5"));
        assert_eq!(
            std::fs::canonicalize(created.parent().unwrap()).unwrap(),
            std::fs::canonicalize(&directory).unwrap()
        );
        std::fs::remove_dir_all(&root).unwrap();
    }

    /// HDF5 creates, writes and reads a file through the extended-length form
    /// of a path longer than MAX_PATH.
    #[test]
    fn hdf5_writes_and_reads_a_file_through_the_extended_length_form() {
        let _guard = crate::hdf5_plugin_isolation::HDF5_TEST_LOCK
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let root =
            std::env::temp_dir().join(format!("insar-native-long-hdf5-{}", std::process::id()));
        let mut directory = root.clone();
        // Grow past MAX_PATH before the file name is added: stopping at 250
        // left 250 or 251 characters for some TEMP lengths, and the
        // nine-character "\long.h5" then landed at or under 260.
        while directory.as_os_str().len() < 260 {
            directory.push("h".repeat(40));
        }
        std::fs::create_dir_all(&directory).unwrap();
        let path = directory.join("long.h5");
        assert!(path.as_os_str().len() > 260);
        {
            let file = hdf5::File::create(native_path(&path)).unwrap();
            file.new_dataset_builder()
                .with_data(&[1_u16, 2, 3])
                .create("values")
                .unwrap();
        }
        assert!(
            path.is_file(),
            "the file exists under its original long path"
        );
        let file = hdf5::File::open(native_path(&path)).unwrap();
        assert_eq!(
            file.dataset("values").unwrap().read_raw::<u16>().unwrap(),
            vec![1, 2, 3]
        );
        drop(file);
        std::fs::remove_dir_all(&root).unwrap();
    }
}
