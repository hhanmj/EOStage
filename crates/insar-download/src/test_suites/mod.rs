#[path = "../../tests/support/nisar_hdf5.rs"]
#[allow(dead_code)]
pub(crate) mod nisar_hdf5_support;
#[path = "../../tests/support/nisar.rs"]
#[allow(dead_code)]
pub(crate) mod nisar_support;

mod artifact_validation;
mod asf_nisar_complete_hdf5;
pub(crate) mod nisar_hdf5_validation;
#[path = "../../tests/support/nisar_ssar_hdf5.rs"]
#[allow(dead_code)]
pub(crate) mod nisar_ssar_hdf5_support;
mod nisar_ssar_hdf5_validation;
