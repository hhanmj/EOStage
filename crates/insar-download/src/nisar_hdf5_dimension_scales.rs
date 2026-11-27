//! Narrow safe boundary for the HDF5 high-level dimension-scale API.
//!
//! The rest of the download/writer code never handles raw HDF5 identifiers or
//! callbacks for dimension scales. Every string and collection returned by
//! this module is bounded, UTF-8 checked, and executed under the crate's HDF5
//! global synchronization boundary.

use std::ffi::{c_char, c_int, c_uint, c_void, CStr, CString};

use hdf5::{Dataset, File};
use hdf5_sys::h5::herr_t;
#[cfg(test)]
use hdf5_sys::h5a::H5Adelete;
use hdf5_sys::h5i::{hid_t, H5Iget_name};

const MAX_DIMENSIONS: usize = 8;
const MAX_SCALES_PER_DIMENSION: usize = 8;
const MAX_SCALE_STRING_BYTES: usize = 1024;

type H5DsIterateCallback =
    Option<unsafe extern "C" fn(hid_t, c_uint, hid_t, *mut c_void) -> herr_t>;

unsafe extern "C" {
    fn H5DSis_scale(dataset_id: hid_t) -> c_int;
    fn H5DSset_scale(dataset_id: hid_t, dimension_name: *const c_char) -> herr_t;
    fn H5DSget_scale_name(dataset_id: hid_t, name: *mut c_char, size: usize) -> isize;
    fn H5DSget_num_scales(dataset_id: hid_t, dimension: c_uint) -> c_int;
    fn H5DSiterate_scales(
        dataset_id: hid_t,
        dimension: c_uint,
        scale_index: *mut c_int,
        visitor: H5DsIterateCallback,
        visitor_data: *mut c_void,
    ) -> herr_t;
    fn H5DSattach_scale(dataset_id: hid_t, scale_id: hid_t, dimension: c_uint) -> herr_t;
    fn H5DSget_label(
        dataset_id: hid_t,
        dimension: c_uint,
        label: *mut c_char,
        size: usize,
    ) -> isize;
    fn H5DSset_label(dataset_id: hid_t, dimension: c_uint, label: *const c_char) -> herr_t;
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct DimensionScaleGraphV1 {
    pub(crate) labels: Vec<String>,
    pub(crate) scales: Vec<Vec<String>>,
    pub(crate) scale_name: Option<String>,
    pub(crate) is_scale: bool,
}

pub(crate) fn read_graph(dataset: &Dataset) -> Result<DimensionScaleGraphV1, ()> {
    let rank = dataset.ndim();
    if rank > MAX_DIMENSIONS {
        return Err(());
    }
    let is_scale = hdf5::sync::sync(|| {
        // SAFETY: `dataset.id()` remains valid for the synchronous call and no
        // pointer arguments are involved.
        unsafe { H5DSis_scale(dataset.id()) }
    });
    if is_scale < 0 {
        return Err(());
    }
    let scale_name = if is_scale > 0 {
        // A dimension scale without a NAME attribute is legal HDF5 and is what
        // the real NISAR RSLC product uses for geolocation-grid scales. The
        // caller still enforces 1-D shape and exact length; the name is an
        // optional label, so an empty name is preserved as `None` instead of
        // rejecting the source.
        read_optional_string(|buffer, size| {
            // SAFETY: the bounded buffer is owned by `read_optional_string` and
            // remains live for this synchronous call.
            unsafe { H5DSget_scale_name(dataset.id(), buffer, size) }
        })?
    } else {
        None
    };
    let mut labels = Vec::with_capacity(rank);
    let mut scales = Vec::with_capacity(rank);
    for dimension in 0..rank {
        labels.push(
            read_optional_string(|buffer, size| {
                // SAFETY: the dimension is bounded by the dataset rank and the
                // output buffer follows the API's queried length.
                unsafe { H5DSget_label(dataset.id(), dimension as c_uint, buffer, size) }
            })?
            .unwrap_or_default(),
        );
        let expected = hdf5::sync::sync(|| {
            // SAFETY: dataset and dimension are validated above.
            unsafe { H5DSget_num_scales(dataset.id(), dimension as c_uint) }
        });
        if expected < 0 || expected as usize > MAX_SCALES_PER_DIMENSION {
            return Err(());
        }
        let mut paths = Vec::with_capacity(expected as usize);
        if expected > 0 {
            let mut index: c_int = 0;
            let status = hdf5::sync::sync(|| {
                // SAFETY: `paths` remains alive and exclusively borrowed for
                // the synchronous iteration; the callback catches panics and
                // validates every returned object path.
                unsafe {
                    H5DSiterate_scales(
                        dataset.id(),
                        dimension as c_uint,
                        &mut index,
                        Some(collect_scale_path),
                        (&mut paths as *mut Vec<String>).cast(),
                    )
                }
            });
            if status < 0 || paths.len() != expected as usize {
                return Err(());
            }
            paths.sort();
        }
        scales.push(paths);
    }
    Ok(DimensionScaleGraphV1 {
        labels,
        scales,
        scale_name,
        is_scale: is_scale > 0,
    })
}

#[cfg(test)]
pub(crate) fn remove_scale_name_for_test(dataset: &Dataset) -> Result<(), ()> {
    let name = c"NAME";
    let status = hdf5::sync::sync(|| {
        // SAFETY: the dataset identifier and static C string remain valid for
        // this synchronous test-only attribute deletion.
        unsafe { H5Adelete(dataset.id(), name.as_ptr()) }
    });
    (status >= 0).then_some(()).ok_or(())
}

pub(crate) fn set_scale(dataset: &Dataset, name: &str) -> Result<(), ()> {
    let name = bounded_c_string(name)?;
    let status = hdf5::sync::sync(|| {
        // SAFETY: dataset and C string remain valid for the synchronous call.
        unsafe { H5DSset_scale(dataset.id(), name.as_ptr()) }
    });
    (status >= 0).then_some(()).ok_or(())
}

/// Marks `dataset` as a dimension scale while preserving an absent source NAME.
///
/// `H5DSset_scale` explicitly accepts a null `dimension_name`; this creates the required
/// CLASS marker without creating a NAME attribute. Calling `set_scale` with an
/// empty string is not equivalent because it invents source metadata.
pub(crate) fn set_scale_unnamed(dataset: &Dataset) -> Result<(), ()> {
    let status = hdf5::sync::sync(|| {
        // SAFETY: `dataset.id()` remains valid for this synchronous call and
        // H5DSset_scale documents a null name pointer as an unnamed scale.
        unsafe { H5DSset_scale(dataset.id(), std::ptr::null()) }
    });
    (status >= 0).then_some(()).ok_or(())
}

pub(crate) fn set_label(dataset: &Dataset, dimension: usize, label: &str) -> Result<(), ()> {
    if dimension >= dataset.ndim() || dimension >= MAX_DIMENSIONS {
        return Err(());
    }
    let label = bounded_c_string(label)?;
    let status = hdf5::sync::sync(|| {
        // SAFETY: dimension is within rank and both arguments remain live.
        unsafe { H5DSset_label(dataset.id(), dimension as c_uint, label.as_ptr()) }
    });
    (status >= 0).then_some(()).ok_or(())
}

pub(crate) fn attach_scale(
    file: &File,
    dataset: &Dataset,
    scale_path: &str,
    dimension: usize,
) -> Result<(), ()> {
    if dimension >= dataset.ndim()
        || dimension >= MAX_DIMENSIONS
        || !is_normalized_absolute_hdf5_path(scale_path)
    {
        return Err(());
    }
    let scale = file.dataset(scale_path).map_err(|_| ())?;
    let status = hdf5::sync::sync(|| {
        // SAFETY: both dataset handles and the validated dimension remain live
        // for the synchronous call.
        unsafe { H5DSattach_scale(dataset.id(), scale.id(), dimension as c_uint) }
    });
    (status >= 0).then_some(()).ok_or(())
}

unsafe extern "C" fn collect_scale_path(
    _dataset_id: hid_t,
    _dimension: c_uint,
    scale_id: hid_t,
    visitor_data: *mut c_void,
) -> herr_t {
    std::panic::catch_unwind(|| {
        // SAFETY: HDF5 supplies a live scale identifier for the duration of
        // the callback. The first call queries only the required length.
        let length = unsafe { H5Iget_name(scale_id, std::ptr::null_mut(), 0) };
        if length <= 0 || length as usize > MAX_SCALE_STRING_BYTES {
            return -1;
        }
        let mut bytes = vec![0_u8; length as usize + 1];
        // SAFETY: `bytes` has the queried length plus the terminating NUL.
        let copied = unsafe { H5Iget_name(scale_id, bytes.as_mut_ptr().cast(), bytes.len()) };
        if copied != length || bytes[length as usize] != 0 {
            return -1;
        }
        let path = match CStr::from_bytes_with_nul(&bytes) {
            Ok(value) => match value.to_str() {
                Ok(value) if is_normalized_absolute_hdf5_path(value) => value.to_owned(),
                _ => return -1,
            },
            Err(_) => return -1,
        };
        // SAFETY: `read_graph` passes an exclusive live Vec pointer and HDF5
        // invokes callbacks synchronously under the global lock.
        let paths = unsafe { &mut *visitor_data.cast::<Vec<String>>() };
        if paths.len() >= MAX_SCALES_PER_DIMENSION {
            return -1;
        }
        paths.push(path);
        0
    })
    .unwrap_or(-1)
}

fn is_normalized_absolute_hdf5_path(value: &str) -> bool {
    value.starts_with('/')
        && value.len() > 1
        && !value.ends_with('/')
        && !value.contains('\\')
        && value
            .split('/')
            .skip(1)
            .all(|component| !component.is_empty() && component != "." && component != "..")
}

fn bounded_c_string(value: &str) -> Result<CString, ()> {
    if value.is_empty() || value.len() > MAX_SCALE_STRING_BYTES {
        return Err(());
    }
    CString::new(value.as_bytes()).map_err(|_| ())
}

fn read_optional_string(
    reader: impl Fn(*mut c_char, usize) -> isize,
) -> Result<Option<String>, ()> {
    let length = hdf5::sync::sync(|| reader(std::ptr::null_mut(), 0));
    if length < 0 || length as usize > MAX_SCALE_STRING_BYTES {
        return Err(());
    }
    if length == 0 {
        return Ok(None);
    }
    let mut bytes = vec![0_u8; length as usize + 1];
    let copied = hdf5::sync::sync(|| reader(bytes.as_mut_ptr().cast(), bytes.len()));
    if copied != length || bytes[length as usize] != 0 {
        return Err(());
    }
    CStr::from_bytes_with_nul(&bytes)
        .map_err(|_| ())?
        .to_str()
        .map(str::to_owned)
        .map(Some)
        .map_err(|_| ())
}

#[cfg(test)]
mod tests {
    use super::is_normalized_absolute_hdf5_path;

    #[test]
    fn unnamed_dimension_scale_is_marked_without_inventing_name() {
        let _guard = crate::hdf5_plugin_isolation::HDF5_TEST_LOCK
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        crate::establish_hdf5_plugin_isolation().expect("establish HDF5 test isolation");
        let path = std::env::temp_dir().join(format!(
            "insar-unnamed-dimension-scale-{}-{}.h5",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let file = hdf5::File::create(&path).unwrap();
        let scale = file.new_dataset::<f64>().shape(2).create("scale").unwrap();
        super::set_scale_unnamed(&scale).unwrap();
        let graph = super::read_graph(&scale).unwrap();
        assert!(graph.is_scale);
        assert_eq!(graph.scale_name, None);
        assert!(scale.attr("NAME").is_err());
        drop(scale);
        drop(file);
        std::fs::remove_file(path).unwrap();
    }

    #[test]
    fn dimension_scale_paths_are_absolute_and_normalized() {
        assert!(is_normalized_absolute_hdf5_path(
            "/science/LSAR/RSLC/metadata/geolocationGrid/zeroDopplerTime"
        ));
        for invalid in [
            "science/LSAR/scale",
            "/",
            "/science//scale",
            "/science/./scale",
            "/science/../scale",
            "/science/scale/",
            "/science\\scale",
        ] {
            assert!(!is_normalized_absolute_hdf5_path(invalid));
        }
    }
}
