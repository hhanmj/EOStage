#[cfg(test)]
use std::cell::Cell;
use std::ffi::CString;
use std::fs::File as StdFile;
use std::io::{Read, Seek, SeekFrom};
use std::path::Path;
use std::ptr;
use std::time::{Duration, Instant};

use hdf5::plist::dataset_create::Layout;
use hdf5::types::{
    CompoundField, CompoundType, FixedAscii, FixedUnicode, FloatSize, H5Type, TypeDescriptor,
};
use hdf5::{Dataset, Datatype, File, Group, LocationToken, LocationType};
use hdf5_sys::h5::{hsize_t, H5_index_t::H5_INDEX_NAME, H5_iter_order_t::H5_ITER_INC};
use hdf5_sys::h5g::{H5G_info_t, H5Gget_info};
use hdf5_sys::h5i::hid_t;
use hdf5_sys::h5l::{
    H5L_info_t, H5Lexists, H5Lget_info_by_idx2, H5Lget_name_by_idx, H5L_TYPE_HARD,
};
use hdf5_sys::h5p::{
    H5Pget_chunk, H5Pget_external_count, H5Pget_filter2, H5Pget_nfilters, H5P_DEFAULT,
};
use hdf5_sys::h5z::{H5Z_filter_t, H5Z_FILTER_DEFLATE, H5Z_FILTER_FLETCHER32, H5Z_FILTER_SHUFFLE};

use crate::artifact_validation::{
    ArtifactValidationContext, ArtifactValidationError, ArtifactValidationIssue,
    ArtifactValidatorId, CompletedArtifactValidator, NisarFrequency, NisarRslcValidationContext,
    NISAR_RSLC_STRUCTURE_VALIDATOR_ID, NISAR_RSLC_STRUCTURE_VALIDATOR_V2_VERSION,
    NISAR_RSLC_STRUCTURE_VALIDATOR_VERSION,
};
use crate::ResolvedAsset;

const HDF5_SIGNATURE: [u8; 8] = [0x89, b'H', b'D', b'F', 0x0d, 0x0a, 0x1a, 0x0a];
const MAX_VISITED_LINKS: usize = 4_096;
const MAX_GROUPS: usize = 512;
const MAX_DATASETS: usize = 3_584;
const MAX_DEPTH: usize = 12;
const MAX_DIRECT_MEMBERS: usize = 512;
const MAX_NAME_BYTES: usize = 512;
const MAX_LINK_NAME_BYTES: usize = 256 * 1024;
const MAX_ATTRIBUTES: usize = 64;
const MAX_RANK: usize = 8;
const MAX_DIMENSION: usize = u32::MAX as usize;
const MAX_FILTERS: i32 = 3;
const MAX_VALUE_ELEMENTS: usize = 16;
const MAX_METADATA_STRING_BYTES: usize = 64 * 1024;
const MAX_METADATA_READ_BYTES: usize = 64 * 1024;
const DEFAULT_DEADLINE: Duration = Duration::from_secs(2);

const IDENTIFICATION_PATH: &str = "/science/LSAR/identification";
const SWATHS_PATH: &str = "/science/LSAR/RSLC/swaths";
const GEOLOCATION_PATH: &str = "/science/LSAR/RSLC/metadata/geolocationGrid";

#[cfg(test)]
thread_local! {
    static READ_1D_CALLS: Cell<usize> = const { Cell::new(0) };
}

#[derive(Clone, Copy, Debug)]
pub(crate) struct NisarHdf5Validator {
    deadline: Duration,
    version: u32,
}

impl NisarHdf5Validator {
    pub(crate) const fn new() -> Self {
        Self {
            deadline: DEFAULT_DEADLINE,
            version: NISAR_RSLC_STRUCTURE_VALIDATOR_VERSION,
        }
    }

    pub(crate) const fn new_v2() -> Self {
        Self {
            deadline: DEFAULT_DEADLINE,
            version: NISAR_RSLC_STRUCTURE_VALIDATOR_V2_VERSION,
        }
    }

    #[doc(hidden)]
    #[cfg(test)]
    pub(crate) const fn with_deadline_for_test(deadline: Duration) -> Self {
        Self {
            deadline,
            version: NISAR_RSLC_STRUCTURE_VALIDATOR_VERSION,
        }
    }

    fn validate_context(
        &self,
        candidate_path: &Path,
        context: &NisarRslcValidationContext,
    ) -> Result<(), ArtifactValidationError> {
        if !crate::hdf5_plugin_loading_disabled() {
            return Err(ArtifactValidationError::unavailable(
                ArtifactValidationIssue::PluginIsolationUnavailable,
            ));
        }

        let deadline = Deadline::new(self.deadline)?;
        check_signature(candidate_path)?;
        deadline.check()?;
        let file =
            crate::hdf5_file::open(candidate_path).map_err(|error| hdf5_open_error(&error))?;
        if !file.is_read_only() {
            return Err(invalid(ArtifactValidationIssue::Hdf5Open));
        }
        deadline.check()?;

        traverse_and_validate_security(&file, &deadline)?;
        validate_required_structure(&file, context, &deadline)
    }
}

pub(crate) fn validate_derived_hdf5_security(
    candidate_path: &Path,
) -> Result<(), ArtifactValidationError> {
    if !crate::hdf5_plugin_loading_disabled() {
        return Err(ArtifactValidationError::unavailable(
            ArtifactValidationIssue::PluginIsolationUnavailable,
        ));
    }
    let deadline = Deadline::new(DEFAULT_DEADLINE)?;
    check_signature(candidate_path)?;
    deadline.check()?;
    let file = crate::hdf5_file::open(candidate_path).map_err(|error| hdf5_open_error(&error))?;
    if !file.is_read_only() {
        return Err(invalid(ArtifactValidationIssue::Hdf5Open));
    }
    traverse_and_validate_security(&file, &deadline)
}

impl Default for NisarHdf5Validator {
    fn default() -> Self {
        Self::new()
    }
}

impl CompletedArtifactValidator for NisarHdf5Validator {
    fn id(&self) -> ArtifactValidatorId {
        NISAR_RSLC_STRUCTURE_VALIDATOR_ID
    }

    fn version(&self) -> u32 {
        self.version
    }

    fn validate(
        &self,
        candidate_path: &Path,
        _asset: &ResolvedAsset,
        context: &ArtifactValidationContext,
    ) -> Result<(), ArtifactValidationError> {
        let context = context.as_nisar_rslc().ok_or_else(|| {
            ArtifactValidationError::unavailable(ArtifactValidationIssue::InvalidContext)
        })?;
        self.validate_context(candidate_path, context)
    }
}

fn invalid(issue: ArtifactValidationIssue) -> ArtifactValidationError {
    ArtifactValidationError::structure_invalid(issue)
}

/// HDF5 judges a file only when it could open it. When the operating system
/// refused the open (HDF5's file driver then reports the OS errno), the bytes
/// were never read: that is a local read failure a retry may resolve, not a
/// structure verdict (L3 gate 2026-10-19).
pub(crate) fn hdf5_open_error(error: &hdf5::Error) -> ArtifactValidationError {
    if hdf5_open_refused_by_os(error) {
        ArtifactValidationError::local_read_failed(ArtifactValidationIssue::Hdf5Open)
    } else {
        invalid(ArtifactValidationIssue::Hdf5Open)
    }
}

/// Whether HDF5's file driver failed to open the file at the operating-system
/// level ("unable to open file: name = '...', errno = N, ..." of the POSIX and
/// Windows drivers), as opposed to failing on the file's contents.
pub(crate) fn hdf5_open_refused_by_os(error: &hdf5::Error) -> bool {
    let hdf5::Error::HDF5(stack) = error else {
        return false;
    };
    stack.clone().expand().is_ok_and(|frames| {
        frames.iter().any(|frame| {
            frame.desc().starts_with("unable to open file: name =")
                && frame.desc().contains("errno =")
        })
    })
}

pub(crate) struct Deadline {
    expires_at: Instant,
}

impl Deadline {
    pub(crate) fn new(duration: Duration) -> Result<Self, ArtifactValidationError> {
        Instant::now()
            .checked_add(duration)
            .map(|expires_at| Self { expires_at })
            .ok_or_else(|| invalid(ArtifactValidationIssue::DeadlineExceeded))
    }

    pub(crate) fn until(expires_at: Instant) -> Self {
        Self { expires_at }
    }

    pub(crate) fn check(&self) -> Result<(), ArtifactValidationError> {
        if Instant::now() >= self.expires_at {
            Err(invalid(ArtifactValidationIssue::DeadlineExceeded))
        } else {
            Ok(())
        }
    }
}

fn check_signature(path: &Path) -> Result<(), ArtifactValidationError> {
    // A candidate the operating system cannot open or read is a local read
    // failure; only a file too short for the signature is a structure verdict.
    let mut file = StdFile::open(path).map_err(|_| {
        ArtifactValidationError::local_read_failed(ArtifactValidationIssue::CandidateIo)
    })?;
    file.seek(SeekFrom::Start(0)).map_err(|_| {
        ArtifactValidationError::local_read_failed(ArtifactValidationIssue::CandidateIo)
    })?;
    let mut signature = [0_u8; HDF5_SIGNATURE.len()];
    file.read_exact(&mut signature).map_err(|error| {
        if error.kind() == std::io::ErrorKind::UnexpectedEof {
            invalid(ArtifactValidationIssue::InvalidSignature)
        } else {
            ArtifactValidationError::local_read_failed(ArtifactValidationIssue::CandidateIo)
        }
    })?;
    if signature != HDF5_SIGNATURE {
        return Err(invalid(ArtifactValidationIssue::InvalidSignature));
    }
    Ok(())
}

#[derive(Clone)]
struct PendingGroup {
    group: Group,
    path: String,
    depth: usize,
}

struct TraversalState {
    links: usize,
    groups: usize,
    datasets: usize,
    link_name_bytes: usize,
    seen: Vec<(u64, LocationToken)>,
}

fn traverse_and_validate_security(
    file: &File,
    deadline: &Deadline,
) -> Result<(), ArtifactValidationError> {
    let root = file
        .group("/")
        .map_err(|_| invalid(ArtifactValidationIssue::Hdf5Open))?;
    let root_info = root
        .loc_info()
        .map_err(|_| invalid(ArtifactValidationIssue::Hdf5Open))?;
    if root_info.num_attrs > MAX_ATTRIBUTES {
        return Err(invalid(ArtifactValidationIssue::AttributeLimit));
    }
    let mut state = TraversalState {
        links: 0,
        groups: 1,
        datasets: 0,
        link_name_bytes: 0,
        seen: vec![(root_info.fileno, root_info.token)],
    };
    let mut pending = vec![PendingGroup {
        group: root,
        path: "/".to_owned(),
        depth: 0,
    }];

    while let Some(parent) = pending.pop() {
        deadline.check()?;
        let members = bounded_hard_link_names(&parent.group, deadline, &mut state.link_name_bytes)?;

        state.links = state
            .links
            .checked_add(members.len())
            .filter(|count| *count <= MAX_VISITED_LINKS)
            .ok_or_else(|| invalid(ArtifactValidationIssue::TraversalLimit))?;
        for name in members {
            deadline.check()?;
            let depth = parent.depth + 1;
            if depth > MAX_DEPTH {
                return Err(invalid(ArtifactValidationIssue::TraversalLimit));
            }
            let info = parent
                .group
                .loc_info_by_name(&name)
                .map_err(|_| invalid(ArtifactValidationIssue::UnsupportedObject))?;
            if info.num_attrs > MAX_ATTRIBUTES {
                return Err(invalid(ArtifactValidationIssue::AttributeLimit));
            }
            if state
                .seen
                .iter()
                .any(|(fileno, token)| *fileno == info.fileno && *token == info.token)
            {
                return Err(invalid(ArtifactValidationIssue::ObjectAliasOrCycle));
            }
            state.seen.push((info.fileno, info.token));
            let path = child_path(&parent.path, &name);
            match info.loc_type {
                LocationType::Group => {
                    state.groups += 1;
                    if state.groups > MAX_GROUPS {
                        return Err(invalid(ArtifactValidationIssue::TraversalLimit));
                    }
                    let group = parent
                        .group
                        .group(&name)
                        .map_err(|_| invalid(ArtifactValidationIssue::UnsupportedObject))?;
                    pending.push(PendingGroup { group, path, depth });
                }
                LocationType::Dataset => {
                    state.datasets += 1;
                    if state.datasets > MAX_DATASETS {
                        return Err(invalid(ArtifactValidationIssue::TraversalLimit));
                    }
                    let dataset = parent
                        .group
                        .dataset(&name)
                        .map_err(|_| invalid(ArtifactValidationIssue::UnsupportedObject))?;
                    inspect_dataset_security(&dataset, deadline)?;
                }
                _ => return Err(invalid(ArtifactValidationIssue::UnsupportedObject)),
            }
        }
    }
    Ok(())
}

fn bounded_hard_link_names(
    group: &Group,
    deadline: &Deadline,
    link_name_bytes: &mut usize,
) -> Result<Vec<String>, ArtifactValidationError> {
    let mut group_info = H5G_info_t::default();
    let status = hdf5::sync::sync(|| unsafe { H5Gget_info(group.id(), &mut group_info) });
    if status < 0 || group_info.nlinks > MAX_DIRECT_MEMBERS as u64 {
        return Err(invalid(ArtifactValidationIssue::TraversalLimit));
    }
    let count = usize::try_from(group_info.nlinks)
        .map_err(|_| invalid(ArtifactValidationIssue::TraversalLimit))?;
    let mut members = Vec::with_capacity(count);
    let current_group = b".\0";

    for index in 0..count {
        deadline.check()?;
        let mut link_info = H5L_info_t::default();
        let status = hdf5::sync::sync(|| unsafe {
            H5Lget_info_by_idx2(
                group.id(),
                current_group.as_ptr().cast(),
                H5_INDEX_NAME,
                H5_ITER_INC,
                index as u64,
                &mut link_info,
                H5P_DEFAULT,
            )
        });
        if status < 0 {
            return Err(invalid(ArtifactValidationIssue::TraversalLimit));
        }
        if link_info.type_ != H5L_TYPE_HARD {
            return Err(invalid(ArtifactValidationIssue::LinkNotHard));
        }

        let name_len = hdf5::sync::sync(|| unsafe {
            H5Lget_name_by_idx(
                group.id(),
                current_group.as_ptr().cast(),
                H5_INDEX_NAME,
                H5_ITER_INC,
                index as u64,
                ptr::null_mut(),
                0,
                H5P_DEFAULT,
            )
        });
        if name_len <= 0 || name_len as usize > MAX_NAME_BYTES {
            return Err(invalid(ArtifactValidationIssue::LinkNameLimit));
        }
        let name_len = usize::try_from(name_len)
            .map_err(|_| invalid(ArtifactValidationIssue::LinkNameLimit))?;
        charge_link_name_bytes(link_name_bytes, name_len)?;
        let buffer_len = name_len
            .checked_add(1)
            .ok_or_else(|| invalid(ArtifactValidationIssue::LinkNameLimit))?;
        let mut buffer = vec![0_u8; buffer_len];
        let copied = hdf5::sync::sync(|| unsafe {
            H5Lget_name_by_idx(
                group.id(),
                current_group.as_ptr().cast(),
                H5_INDEX_NAME,
                H5_ITER_INC,
                index as u64,
                buffer.as_mut_ptr().cast(),
                buffer.len(),
                H5P_DEFAULT,
            )
        });
        if copied < 0 || copied as usize != name_len || buffer[name_len] != 0 {
            return Err(invalid(ArtifactValidationIssue::LinkNameLimit));
        }
        let name = std::str::from_utf8(&buffer[..name_len])
            .map_err(|_| invalid(ArtifactValidationIssue::LinkNameLimit))?;
        if name.as_bytes().contains(&0) {
            return Err(invalid(ArtifactValidationIssue::LinkNameLimit));
        }
        members.push(name.to_owned());
    }
    Ok(members)
}

fn charge_link_name_bytes(
    link_name_bytes: &mut usize,
    name_len: usize,
) -> Result<(), ArtifactValidationError> {
    let charged = link_name_bytes
        .checked_add(name_len)
        .filter(|bytes| *bytes <= MAX_LINK_NAME_BYTES)
        .ok_or_else(|| invalid(ArtifactValidationIssue::LinkNameLimit))?;
    *link_name_bytes = charged;
    Ok(())
}

fn child_path(parent: &str, name: &str) -> String {
    if parent == "/" {
        format!("/{name}")
    } else {
        format!("{parent}/{name}")
    }
}

fn inspect_dataset_security(
    dataset: &Dataset,
    deadline: &Deadline,
) -> Result<(), ArtifactValidationError> {
    let _ = bounded_extents(dataset, ArtifactValidationIssue::DatasetShape)?;
    deadline.check()?;
    let dcpl = dataset
        .dcpl()
        .map_err(|_| invalid(ArtifactValidationIssue::DatasetShape))?;
    let layout = dcpl
        .get_layout()
        .map_err(|_| invalid(ArtifactValidationIssue::DatasetVirtualLayout))?;
    if layout == Layout::Virtual {
        return Err(invalid(ArtifactValidationIssue::DatasetVirtualLayout));
    }
    let external_count = hdf5::sync::sync(|| unsafe { H5Pget_external_count(dcpl.id()) });
    if external_count != 0 {
        return Err(invalid(ArtifactValidationIssue::DatasetExternalStorage));
    }
    let datatype_size = dataset
        .dtype()
        .map_err(|_| invalid(ArtifactValidationIssue::DatasetFilter))?
        .size();
    validate_filter_pipeline(dcpl.id(), datatype_size)?;
    deadline.check()
}

fn validate_filter_pipeline(
    dcpl_id: hid_t,
    datatype_size: usize,
) -> Result<(), ArtifactValidationError> {
    let filter_count = hdf5::sync::sync(|| unsafe { H5Pget_nfilters(dcpl_id) });
    if !(0..=MAX_FILTERS).contains(&filter_count) {
        return Err(invalid(ArtifactValidationIssue::DatasetFilter));
    }
    let mut seen = 0_u8;
    for index in 0..filter_count as u32 {
        let mut client_value_count = 0_usize;
        let filter_id = hdf5::sync::sync(|| unsafe {
            H5Pget_filter2(
                dcpl_id,
                index,
                ptr::null_mut(),
                &mut client_value_count,
                ptr::null_mut(),
                0,
                ptr::null_mut(),
                ptr::null_mut(),
            )
        });
        let Some((bit, expected_client_value_count)) = filter_contract(filter_id) else {
            return Err(invalid(ArtifactValidationIssue::DatasetFilter));
        };
        if client_value_count != expected_client_value_count || seen & bit != 0 {
            return Err(invalid(ArtifactValidationIssue::DatasetFilter));
        }
        seen |= bit;

        let mut values = [0_u32; 1];
        if expected_client_value_count == 1 {
            let mut returned_count = expected_client_value_count;
            let confirmed_id = hdf5::sync::sync(|| unsafe {
                H5Pget_filter2(
                    dcpl_id,
                    index,
                    ptr::null_mut(),
                    &mut returned_count,
                    values.as_mut_ptr(),
                    0,
                    ptr::null_mut(),
                    ptr::null_mut(),
                )
            });
            if confirmed_id != filter_id || returned_count != expected_client_value_count {
                return Err(invalid(ArtifactValidationIssue::DatasetFilter));
            }
        }
        if !filter_client_values_are_valid(
            filter_id,
            &values[..expected_client_value_count],
            datatype_size,
        ) {
            return Err(invalid(ArtifactValidationIssue::DatasetFilter));
        }
    }
    Ok(())
}

fn filter_contract(filter_id: H5Z_filter_t) -> Option<(u8, usize)> {
    match filter_id {
        H5Z_FILTER_SHUFFLE => Some((1, 1)),
        H5Z_FILTER_DEFLATE => Some((2, 1)),
        H5Z_FILTER_FLETCHER32 => Some((4, 0)),
        _ => None,
    }
}

#[cfg(test)]
fn filter_id_bit(filter_id: H5Z_filter_t) -> Option<u8> {
    filter_contract(filter_id).map(|(bit, _)| bit)
}

fn filter_client_values_are_valid(
    filter_id: H5Z_filter_t,
    values: &[u32],
    datatype_size: usize,
) -> bool {
    match (filter_id, values) {
        (H5Z_FILTER_SHUFFLE, [element_size]) => {
            u32::try_from(datatype_size).ok() == Some(*element_size) && *element_size != 0
        }
        (H5Z_FILTER_DEFLATE, [level]) => *level <= 9,
        (H5Z_FILTER_FLETCHER32, []) => true,
        _ => false,
    }
}

pub(crate) fn checked_link_exists(
    location_id: hid_t,
    name: &str,
    issue: ArtifactValidationIssue,
) -> Result<bool, ArtifactValidationError> {
    let name = CString::new(name).map_err(|_| invalid(issue))?;
    let status = hdf5::sync::sync(|| unsafe { H5Lexists(location_id, name.as_ptr(), H5P_DEFAULT) });
    if status < 0 {
        Err(invalid(issue))
    } else {
        Ok(status > 0)
    }
}

#[derive(Clone, Debug)]
pub(crate) struct DatasetExtents {
    pub(crate) scalar: bool,
    pub(crate) dims: Vec<usize>,
    pub(crate) max_dims: Vec<Option<usize>>,
}

pub(crate) fn bounded_extents(
    dataset: &Dataset,
    issue: ArtifactValidationIssue,
) -> Result<DatasetExtents, ArtifactValidationError> {
    let extents = dataset
        .space()
        .and_then(|space| space.extents())
        .map_err(|_| invalid(issue))?;
    if extents.is_null() || extents.ndim() > MAX_RANK {
        return Err(invalid(issue));
    }
    let dims = extents.dims();
    let max_dims = extents.maxdims();
    if dims.len() != max_dims.len()
        || dims.iter().any(|dimension| *dimension > MAX_DIMENSION)
        || max_dims
            .iter()
            .zip(&dims)
            .any(|(maximum, current)| maximum.is_some_and(|maximum| maximum < *current))
    {
        return Err(invalid(issue));
    }
    Ok(DatasetExtents {
        scalar: extents.is_scalar(),
        dims,
        max_dims,
    })
}

fn validate_metadata_read_budget(
    dataset: &Dataset,
    extents: &DatasetExtents,
    datatype_width: usize,
) -> Result<(), ArtifactValidationError> {
    let issue = ArtifactValidationIssue::DatasetReadBudget;
    if datatype_width == 0
        || extents.max_dims.len() != extents.dims.len()
        || extents
            .max_dims
            .iter()
            .zip(&extents.dims)
            .any(|(maximum, current)| *maximum != Some(*current))
    {
        return Err(invalid(issue));
    }

    let dcpl = dataset.dcpl().map_err(|_| invalid(issue))?;
    match dcpl.get_layout().map_err(|_| invalid(issue))? {
        Layout::Compact | Layout::Contiguous => {}
        Layout::Chunked => {
            let mut chunk_dims: [hsize_t; MAX_RANK] = [0; MAX_RANK];
            let chunk_rank = hdf5::sync::sync(|| unsafe {
                H5Pget_chunk(dcpl.id(), MAX_RANK as i32, chunk_dims.as_mut_ptr())
            });
            validate_chunk_read_budget(chunk_rank, &chunk_dims, &extents.dims, datatype_width)?;
        }
        Layout::Virtual => return Err(invalid(issue)),
    }
    // For filtered chunks HDF5 allocates the file-controlled stored chunk size before
    // unfiltering, so nominal dimensions alone cannot enforce this in-process budget.
    let filter_count = hdf5::sync::sync(|| unsafe { H5Pget_nfilters(dcpl.id()) });
    if filter_count != 0 {
        return Err(invalid(issue));
    }

    validate_logical_read_budget(extents.scalar, &extents.dims, datatype_width)
}

fn validate_logical_read_budget(
    scalar: bool,
    current_dims: &[usize],
    datatype_width: usize,
) -> Result<(), ArtifactValidationError> {
    let issue = ArtifactValidationIssue::DatasetReadBudget;
    let elements = if scalar {
        Some(1)
    } else {
        current_dims
            .iter()
            .try_fold(1_usize, |total, dimension| total.checked_mul(*dimension))
    };
    elements
        .and_then(|elements| elements.checked_mul(datatype_width))
        .filter(|bytes| *bytes <= MAX_METADATA_READ_BYTES)
        .map(|_| ())
        .ok_or_else(|| invalid(issue))
}

fn validate_chunk_read_budget(
    chunk_rank: i32,
    chunk_dims: &[hsize_t; MAX_RANK],
    current_dims: &[usize],
    datatype_width: usize,
) -> Result<(), ArtifactValidationError> {
    let issue = ArtifactValidationIssue::DatasetReadBudget;
    let rank = usize::try_from(chunk_rank)
        .ok()
        .filter(|rank| *rank > 0 && *rank <= MAX_RANK && *rank == current_dims.len())
        .ok_or_else(|| invalid(issue))?;
    let chunk_elements = chunk_dims[..rank]
        .iter()
        .zip(current_dims)
        .try_fold(1_usize, |elements, (chunk_dimension, current_dimension)| {
            let chunk_dimension = usize::try_from(*chunk_dimension).ok()?;
            if chunk_dimension == 0 || chunk_dimension > *current_dimension {
                return None;
            }
            elements.checked_mul(chunk_dimension)
        })
        .ok_or_else(|| invalid(issue))?;
    chunk_elements
        .checked_mul(datatype_width)
        .filter(|bytes| *bytes <= MAX_METADATA_READ_BYTES)
        .map(|_| ())
        .ok_or_else(|| invalid(issue))
}

fn validate_required_structure(
    file: &File,
    context: &NisarRslcValidationContext,
    deadline: &Deadline,
) -> Result<(), ArtifactValidationError> {
    for path in [
        "/science",
        "/science/LSAR",
        IDENTIFICATION_PATH,
        "/science/LSAR/RSLC",
        SWATHS_PATH,
    ] {
        file.group(path)
            .map_err(|_| invalid(ArtifactValidationIssue::RequiredObjectMissing))?;
        deadline.check()?;
    }
    if checked_link_exists(
        file.id(),
        "/science/SSAR",
        ArtifactValidationIssue::IdentificationMismatch,
    )? || checked_link_exists(
        file.id(),
        "/science/LSAR/SLC",
        ArtifactValidationIssue::IdentificationMismatch,
    )? {
        return Err(invalid(ArtifactValidationIssue::IdentificationMismatch));
    }

    let identification = file
        .group(IDENTIFICATION_PATH)
        .map_err(|_| invalid(ArtifactValidationIssue::RequiredObjectMissing))?;
    let mut strings = StringBudget::default();
    for (name, expected) in [
        ("missionId", "NISAR"),
        ("productType", "RSLC"),
        ("productLevel", "L1"),
        ("radarBand", "L"),
        (
            "productSpecificationVersion",
            context.product_specification_version(),
        ),
        ("granuleId", context.granule_id()),
    ] {
        let value = read_fixed_scalar(
            &required_dataset(
                &identification,
                name,
                ArtifactValidationIssue::IdentificationMismatch,
            )?,
            &mut strings,
            ArtifactValidationIssue::IdentificationMismatch,
        )?;
        if value != expected {
            return Err(invalid(ArtifactValidationIssue::IdentificationMismatch));
        }
        deadline.check()?;
    }

    if let Some(expected_crid) = context.crid() {
        let composite_release_id = read_fixed_scalar(
            &required_dataset(
                &identification,
                "compositeReleaseId",
                ArtifactValidationIssue::IdentificationMismatch,
            )?,
            &mut strings,
            ArtifactValidationIssue::IdentificationMismatch,
        )?;
        if composite_release_id != expected_crid {
            return Err(invalid(ArtifactValidationIssue::IdentificationMismatch));
        }
        deadline.check()?;
    }

    let frequencies = read_fixed_list(
        &required_dataset(
            &identification,
            "listOfFrequencies",
            ArtifactValidationIssue::FrequencyMismatch,
        )?,
        &mut strings,
        ArtifactValidationIssue::FrequencyMismatch,
    )?;
    let mut actual_frequencies = frequencies;
    actual_frequencies.sort_unstable();
    let mut expected_frequencies = [NisarFrequency::A, NisarFrequency::B]
        .into_iter()
        .filter(|frequency| context.has_frequency(*frequency))
        .map(|frequency| frequency.as_str().to_owned())
        .collect::<Vec<_>>();
    expected_frequencies.sort_unstable();
    if actual_frequencies != expected_frequencies {
        return Err(invalid(ArtifactValidationIssue::FrequencyMismatch));
    }

    let swaths = file
        .group(SWATHS_PATH)
        .map_err(|_| invalid(ArtifactValidationIssue::RequiredObjectMissing))?;
    for frequency in [NisarFrequency::A, NisarFrequency::B] {
        validate_frequency(&swaths, frequency, context, &mut strings, deadline)?;
    }

    let bounding = read_fixed_scalar(
        &required_dataset(
            &identification,
            "boundingPolygon",
            ArtifactValidationIssue::BoundingPolygonInvalid,
        )?,
        &mut strings,
        ArtifactValidationIssue::BoundingPolygonInvalid,
    )?;
    if bounding.is_empty() {
        return Err(invalid(ArtifactValidationIssue::BoundingPolygonInvalid));
    }
    validate_geolocation(file, deadline)
}

pub(crate) fn required_dataset(
    group: &Group,
    name: &str,
    issue: ArtifactValidationIssue,
) -> Result<Dataset, ArtifactValidationError> {
    group.dataset(name).map_err(|_| invalid(issue))
}

fn validate_frequency(
    swaths: &Group,
    frequency: NisarFrequency,
    context: &NisarRslcValidationContext,
    strings: &mut StringBudget,
    deadline: &Deadline,
) -> Result<(), ArtifactValidationError> {
    let name = match frequency {
        NisarFrequency::A => "frequencyA",
        NisarFrequency::B => "frequencyB",
    };
    let expected = context.polarizations(frequency);
    if checked_link_exists(
        swaths.id(),
        name,
        ArtifactValidationIssue::FrequencyMismatch,
    )? != expected.is_some()
    {
        return Err(invalid(ArtifactValidationIssue::FrequencyMismatch));
    }
    let Some(expected) = expected else {
        return Ok(());
    };
    let group = swaths
        .group(name)
        .map_err(|_| invalid(ArtifactValidationIssue::FrequencyMismatch))?;
    let mut actual = read_fixed_list(
        &required_dataset(
            &group,
            "listOfPolarizations",
            ArtifactValidationIssue::PolarizationMismatch,
        )?,
        strings,
        ArtifactValidationIssue::PolarizationMismatch,
    )?;
    actual.sort_unstable();
    let mut expected_names = expected
        .iter()
        .map(|polarization| polarization.as_str().to_owned())
        .collect::<Vec<_>>();
    expected_names.sort_unstable();
    if actual != expected_names {
        return Err(invalid(ArtifactValidationIssue::PolarizationMismatch));
    }

    for candidate in ["HH", "HV", "VH", "VV"] {
        let declared = expected_names.iter().any(|value| value == candidate);
        if checked_link_exists(
            group.id(),
            candidate,
            ArtifactValidationIssue::PolarizationMismatch,
        )? != declared
        {
            return Err(invalid(ArtifactValidationIssue::PolarizationMismatch));
        }
        if declared {
            validate_image_dataset(
                &required_dataset(
                    &group,
                    candidate,
                    ArtifactValidationIssue::ImageDatasetInvalid,
                )?,
                deadline,
            )?;
        }
    }
    Ok(())
}

pub(crate) fn validate_image_dataset(
    dataset: &Dataset,
    deadline: &Deadline,
) -> Result<(), ArtifactValidationError> {
    let extents = bounded_extents(dataset, ArtifactValidationIssue::ImageDatasetInvalid)?;
    if extents.scalar || extents.dims.len() != 2 || extents.dims.contains(&0) {
        return Err(invalid(ArtifactValidationIssue::ImageDatasetInvalid));
    }
    let expected = Datatype::from_descriptor(&TypeDescriptor::Compound(CompoundType {
        fields: vec![
            CompoundField::new("r", TypeDescriptor::Float(FloatSize::U4), 0, 0),
            CompoundField::new("i", TypeDescriptor::Float(FloatSize::U4), 4, 1),
        ],
        size: 8,
    }))
    .map_err(|_| invalid(ArtifactValidationIssue::ImageDatasetInvalid))?;
    let actual = dataset
        .dtype()
        .map_err(|_| invalid(ArtifactValidationIssue::ImageDatasetInvalid))?;
    if actual != expected {
        return Err(invalid(ArtifactValidationIssue::ImageDatasetInvalid));
    }
    deadline.check()
}

fn validate_geolocation(file: &File, deadline: &Deadline) -> Result<(), ArtifactValidationError> {
    let group = file
        .group(GEOLOCATION_PATH)
        .map_err(|_| invalid(ArtifactValidationIssue::GeolocationGridInvalid))?;
    let epsg = required_dataset(
        &group,
        "epsg",
        ArtifactValidationIssue::GeolocationGridInvalid,
    )?;
    let epsg_extents = bounded_extents(&epsg, ArtifactValidationIssue::GeolocationGridInvalid)?;
    if !epsg_extents.scalar
        || !epsg
            .dtype()
            .map_err(|_| invalid(ArtifactValidationIssue::GeolocationGridInvalid))?
            .is::<i32>()
    {
        return Err(invalid(ArtifactValidationIssue::GeolocationGridInvalid));
    }

    let coordinate_x = required_float64_shape(&group, "coordinateX", 3)?;
    let coordinate_y = required_float64_shape(&group, "coordinateY", 3)?;
    if coordinate_x != coordinate_y {
        return Err(invalid(ArtifactValidationIssue::GeolocationGridInvalid));
    }
    for name in ["slantRange", "zeroDopplerTime", "heightAboveEllipsoid"] {
        let _ = required_float64_shape(&group, name, 1)?;
        deadline.check()?;
    }
    Ok(())
}

fn required_float64_shape(
    group: &Group,
    name: &str,
    rank: usize,
) -> Result<Vec<usize>, ArtifactValidationError> {
    let dataset = required_dataset(group, name, ArtifactValidationIssue::GeolocationGridInvalid)?;
    let extents = bounded_extents(&dataset, ArtifactValidationIssue::GeolocationGridInvalid)?;
    if extents.scalar
        || extents.dims.len() != rank
        || extents.dims.contains(&0)
        || !dataset
            .dtype()
            .map_err(|_| invalid(ArtifactValidationIssue::GeolocationGridInvalid))?
            .is::<f64>()
    {
        return Err(invalid(ArtifactValidationIssue::GeolocationGridInvalid));
    }
    Ok(extents.dims)
}

#[derive(Clone, Copy)]
enum FixedEncoding {
    Ascii,
    Unicode,
}

#[derive(Default)]
pub(crate) struct StringBudget {
    bytes: usize,
}

impl StringBudget {
    fn charge(&mut self, width: usize, elements: usize) -> Result<(), ArtifactValidationError> {
        let bytes = width
            .checked_mul(elements)
            .and_then(|value| self.bytes.checked_add(value))
            .filter(|value| *value <= MAX_METADATA_STRING_BYTES)
            .ok_or_else(|| invalid(ArtifactValidationIssue::TraversalLimit))?;
        self.bytes = bytes;
        Ok(())
    }
}

fn fixed_string_type(
    dataset: &Dataset,
    issue: ArtifactValidationIssue,
) -> Result<(FixedEncoding, usize), ArtifactValidationError> {
    let dtype = dataset.dtype().map_err(|_| invalid(issue))?;
    let width = dtype.size();
    if width == 0 || width > MAX_METADATA_STRING_BYTES {
        return Err(invalid(issue));
    }
    let ascii = Datatype::from_descriptor(&TypeDescriptor::FixedAscii(width))
        .map_err(|_| invalid(issue))?;
    if dtype == ascii {
        return Ok((FixedEncoding::Ascii, width));
    }
    let unicode = Datatype::from_descriptor(&TypeDescriptor::FixedUnicode(width))
        .map_err(|_| invalid(issue))?;
    if dtype == unicode {
        return Ok((FixedEncoding::Unicode, width));
    }
    Err(invalid(issue))
}

trait FixedBytes {
    fn fixed_bytes(&self) -> &[u8];
}

impl<const N: usize> FixedBytes for FixedAscii<N> {
    fn fixed_bytes(&self) -> &[u8] {
        self.as_bytes()
    }
}

impl<const N: usize> FixedBytes for FixedUnicode<N> {
    fn fixed_bytes(&self) -> &[u8] {
        self.as_bytes()
    }
}

fn decode_fixed<T: FixedBytes>(
    value: &T,
    encoding: FixedEncoding,
    issue: ArtifactValidationIssue,
) -> Result<String, ArtifactValidationError> {
    let bytes = value.fixed_bytes();
    if matches!(encoding, FixedEncoding::Ascii) && !bytes.is_ascii() {
        return Err(invalid(issue));
    }
    std::str::from_utf8(bytes)
        .map(str::to_owned)
        .map_err(|_| invalid(issue))
}

fn read_scalar_t<T: H5Type + FixedBytes>(
    dataset: &Dataset,
    encoding: FixedEncoding,
    issue: ArtifactValidationIssue,
) -> Result<String, ArtifactValidationError> {
    let value = dataset.read_scalar::<T>().map_err(|_| invalid(issue))?;
    decode_fixed(&value, encoding, issue)
}

fn read_list_t<T: H5Type + FixedBytes>(
    dataset: &Dataset,
    encoding: FixedEncoding,
    issue: ArtifactValidationIssue,
) -> Result<Vec<String>, ArtifactValidationError> {
    #[cfg(test)]
    READ_1D_CALLS.with(|calls| calls.set(calls.get().saturating_add(1)));
    let values = dataset.read_1d::<T>().map_err(|_| invalid(issue))?;
    values
        .iter()
        .map(|value| decode_fixed(value, encoding, issue))
        .collect()
}

macro_rules! read_sized_scalar {
    ($dataset:expr, $encoding:expr, $issue:expr, $ty:ident) => {{
        let width = $dataset.dtype().map_err(|_| invalid($issue))?.size();
        match width {
            0..=1 => read_scalar_t::<$ty<1>>($dataset, $encoding, $issue),
            2 => read_scalar_t::<$ty<2>>($dataset, $encoding, $issue),
            3..=4 => read_scalar_t::<$ty<4>>($dataset, $encoding, $issue),
            5..=8 => read_scalar_t::<$ty<8>>($dataset, $encoding, $issue),
            9..=16 => read_scalar_t::<$ty<16>>($dataset, $encoding, $issue),
            17..=32 => read_scalar_t::<$ty<32>>($dataset, $encoding, $issue),
            33..=64 => read_scalar_t::<$ty<64>>($dataset, $encoding, $issue),
            65..=128 => read_scalar_t::<$ty<128>>($dataset, $encoding, $issue),
            129..=256 => read_scalar_t::<$ty<256>>($dataset, $encoding, $issue),
            257..=512 => read_scalar_t::<$ty<512>>($dataset, $encoding, $issue),
            513..=1024 => read_scalar_t::<$ty<1024>>($dataset, $encoding, $issue),
            1025..=2048 => read_scalar_t::<$ty<2048>>($dataset, $encoding, $issue),
            2049..=4096 => read_scalar_t::<$ty<4096>>($dataset, $encoding, $issue),
            4097..=8192 => read_scalar_t::<$ty<8192>>($dataset, $encoding, $issue),
            8193..=16384 => read_scalar_t::<$ty<16384>>($dataset, $encoding, $issue),
            16385..=32768 => read_scalar_t::<$ty<32768>>($dataset, $encoding, $issue),
            _ => read_scalar_t::<$ty<65536>>($dataset, $encoding, $issue),
        }
    }};
}

macro_rules! read_sized_list {
    ($dataset:expr, $encoding:expr, $issue:expr, $ty:ident) => {{
        let width = $dataset.dtype().map_err(|_| invalid($issue))?.size();
        match width {
            0..=1 => read_list_t::<$ty<1>>($dataset, $encoding, $issue),
            2 => read_list_t::<$ty<2>>($dataset, $encoding, $issue),
            3..=4 => read_list_t::<$ty<4>>($dataset, $encoding, $issue),
            5..=8 => read_list_t::<$ty<8>>($dataset, $encoding, $issue),
            9..=16 => read_list_t::<$ty<16>>($dataset, $encoding, $issue),
            17..=32 => read_list_t::<$ty<32>>($dataset, $encoding, $issue),
            33..=64 => read_list_t::<$ty<64>>($dataset, $encoding, $issue),
            65..=128 => read_list_t::<$ty<128>>($dataset, $encoding, $issue),
            129..=256 => read_list_t::<$ty<256>>($dataset, $encoding, $issue),
            257..=512 => read_list_t::<$ty<512>>($dataset, $encoding, $issue),
            513..=1024 => read_list_t::<$ty<1024>>($dataset, $encoding, $issue),
            1025..=2048 => read_list_t::<$ty<2048>>($dataset, $encoding, $issue),
            2049..=4096 => read_list_t::<$ty<4096>>($dataset, $encoding, $issue),
            _ => Err(invalid($issue)),
        }
    }};
}

pub(crate) fn read_fixed_scalar(
    dataset: &Dataset,
    budget: &mut StringBudget,
    issue: ArtifactValidationIssue,
) -> Result<String, ArtifactValidationError> {
    let extents = bounded_extents(dataset, issue)?;
    if !extents.scalar {
        return Err(invalid(issue));
    }
    let (encoding, width) = fixed_string_type(dataset, issue)?;
    validate_metadata_read_budget(dataset, &extents, width)?;
    budget.charge(width, 1)?;
    match encoding {
        FixedEncoding::Ascii => read_sized_scalar!(dataset, encoding, issue, FixedAscii),
        FixedEncoding::Unicode => read_sized_scalar!(dataset, encoding, issue, FixedUnicode),
    }
}

pub(crate) fn read_fixed_list(
    dataset: &Dataset,
    budget: &mut StringBudget,
    issue: ArtifactValidationIssue,
) -> Result<Vec<String>, ArtifactValidationError> {
    let extents = bounded_extents(dataset, issue)?;
    if extents.scalar
        || extents.dims.len() != 1
        || extents.dims[0] == 0
        || extents.dims[0] > MAX_VALUE_ELEMENTS
    {
        return Err(invalid(issue));
    }
    let elements = extents.dims[0];
    let (encoding, width) = fixed_string_type(dataset, issue)?;
    validate_metadata_read_budget(dataset, &extents, width)?;
    budget.charge(width, elements)?;
    match encoding {
        FixedEncoding::Ascii => read_sized_list!(dataset, encoding, issue, FixedAscii),
        FixedEncoding::Unicode => read_sized_list!(dataset, encoding, issue, FixedUnicode),
    }
}

#[cfg(test)]
pub(crate) fn read_1d_call_count_for_test() -> usize {
    READ_1D_CALLS.with(Cell::get)
}

#[cfg(test)]
mod tests {
    use hdf5::filters::Filter;

    use super::{
        charge_link_name_bytes, checked_link_exists, filter_client_values_are_valid, filter_id_bit,
        hsize_t, validate_chunk_read_budget, validate_logical_read_budget, MAX_LINK_NAME_BYTES,
        MAX_RANK,
    };
    use crate::artifact_validation::ArtifactValidationIssue;

    #[test]
    fn filter_allowlist_rejects_unknown_and_unreviewed_builtin_filters() {
        assert!(filter_id_bit(Filter::Shuffle.id()).is_some());
        assert!(filter_id_bit(Filter::Deflate(6).id()).is_some());
        assert!(filter_id_bit(Filter::Fletcher32.id()).is_some());
        assert!(filter_id_bit(Filter::NBit.id()).is_none());
        assert!(filter_id_bit(Filter::User(32_000, Vec::new()).id()).is_none());
    }

    #[test]
    fn allowed_filter_client_values_match_the_bundled_hdf5_contract() {
        assert!(filter_client_values_are_valid(
            Filter::Shuffle.id(),
            &[8],
            8
        ));
        assert!(!filter_client_values_are_valid(
            Filter::Shuffle.id(),
            &[],
            8
        ));
        assert!(!filter_client_values_are_valid(
            Filter::Shuffle.id(),
            &[4],
            8
        ));

        assert!(filter_client_values_are_valid(
            Filter::Deflate(0).id(),
            &[0],
            1
        ));
        assert!(filter_client_values_are_valid(
            Filter::Deflate(9).id(),
            &[9],
            1
        ));
        assert!(!filter_client_values_are_valid(
            Filter::Deflate(6).id(),
            &[],
            1
        ));
        assert!(!filter_client_values_are_valid(
            Filter::Deflate(6).id(),
            &[6, 7],
            1
        ));
        assert!(!filter_client_values_are_valid(
            Filter::Deflate(9).id(),
            &[10],
            1
        ));

        assert!(filter_client_values_are_valid(
            Filter::Fletcher32.id(),
            &[],
            1
        ));
        assert!(!filter_client_values_are_valid(
            Filter::Fletcher32.id(),
            &[1],
            1
        ));
    }

    #[test]
    fn link_existence_errors_are_not_treated_as_absence() {
        assert!(checked_link_exists(
            -1,
            "/unreadable",
            ArtifactValidationIssue::IdentificationMismatch,
        )
        .is_err());
    }

    #[test]
    fn plugin_isolation_requires_the_live_hdf5_control_mask_to_be_zero() {
        let _guard = crate::hdf5_plugin_isolation::HDF5_TEST_LOCK
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        // The validator gate must reflect the actual HDF5 plugin control
        // mask, not an external env sentinel. The fixture-based positive and
        // negative controls live in test_suites/nisar_hdf5_validation.rs;
        // this guard only asserts the exported API is wired to the real mask.
        crate::hdf5_plugin_isolation::crt_env::remove("HDF5_PLUGIN_PRELOAD");
        crate::hdf5_plugin_isolation::reenable_hdf5_plugin_loading_for_test();
        if crate::hdf5_plugin_loading_disabled() {
            crate::establish_hdf5_plugin_isolation().expect("restore isolation");
            return;
        }
        assert!(!crate::hdf5_plugin_loading_disabled());
        crate::establish_hdf5_plugin_isolation().expect("restore isolation");
        assert!(crate::hdf5_plugin_loading_disabled());
    }

    #[test]
    fn link_name_budget_is_charged_before_each_allocation() {
        let mut charged = MAX_LINK_NAME_BYTES - 1;
        charge_link_name_bytes(&mut charged, 1).expect("exact global budget must fit");
        assert_eq!(charged, MAX_LINK_NAME_BYTES);

        let before_rejection = charged;
        let error = charge_link_name_bytes(&mut charged, 1).unwrap_err();
        assert_eq!(error.issue(), ArtifactValidationIssue::LinkNameLimit);
        assert_eq!(charged, before_rejection, "a rejected charge is atomic");

        let mut overflowing = usize::MAX;
        let error = charge_link_name_bytes(&mut overflowing, 1).unwrap_err();
        assert_eq!(error.issue(), ArtifactValidationIssue::LinkNameLimit);
        assert_eq!(overflowing, usize::MAX);
    }

    #[test]
    fn chunk_read_budget_rejects_invalid_rank_dimensions_and_arithmetic() {
        let mut chunk_dims = [0; MAX_RANK];
        chunk_dims[..2].copy_from_slice(&[4, 4]);
        validate_chunk_read_budget(2, &chunk_dims, &[4, 4], 8)
            .expect("small exact-rank chunk must fit");

        for (rank, current_dims) in [(-1, &[4, 4][..]), (0, &[4, 4][..]), (9, &[4, 4][..])] {
            let error = validate_chunk_read_budget(rank, &chunk_dims, current_dims, 8).unwrap_err();
            assert_eq!(error.issue(), ArtifactValidationIssue::DatasetReadBudget);
        }

        let error = validate_chunk_read_budget(1, &chunk_dims, &[4, 4], 8).unwrap_err();
        assert_eq!(error.issue(), ArtifactValidationIssue::DatasetReadBudget);

        chunk_dims[0] = 0;
        let error = validate_chunk_read_budget(1, &chunk_dims, &[1], 8).unwrap_err();
        assert_eq!(error.issue(), ArtifactValidationIssue::DatasetReadBudget);

        chunk_dims[0] = 2;
        let error = validate_chunk_read_budget(1, &chunk_dims, &[1], 8).unwrap_err();
        assert_eq!(error.issue(), ArtifactValidationIssue::DatasetReadBudget);

        chunk_dims[0] = 8_193;
        let error = validate_chunk_read_budget(1, &chunk_dims, &[8_193], 8).unwrap_err();
        assert_eq!(error.issue(), ArtifactValidationIssue::DatasetReadBudget);

        chunk_dims[0] = hsize_t::MAX;
        let error = validate_chunk_read_budget(1, &chunk_dims, &[usize::MAX], 2).unwrap_err();
        assert_eq!(error.issue(), ArtifactValidationIssue::DatasetReadBudget);
    }

    #[test]
    fn logical_read_budget_checks_contiguous_sizes_and_overflow() {
        validate_logical_read_budget(true, &[], 65_536).expect("exact scalar read budget must fit");
        validate_logical_read_budget(false, &[16], 4_096)
            .expect("exact vector read budget must fit");

        for (scalar, dims, width) in [
            (true, &[][..], 65_537),
            (false, &[2][..], 32_769),
            (false, &[usize::MAX, 2][..], 1),
        ] {
            let error = validate_logical_read_budget(scalar, dims, width).unwrap_err();
            assert_eq!(error.issue(), ArtifactValidationIssue::DatasetReadBudget);
        }
    }
}
