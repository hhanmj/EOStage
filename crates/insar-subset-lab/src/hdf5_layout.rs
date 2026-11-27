use std::collections::{BTreeMap, BTreeSet};
use std::fs::{self, File, OpenOptions};
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};

use hdf5::filters::Filter;
use hdf5::plist::dataset_create::Layout;
use hdf5::plist::ObjectCopy;
use hdf5::types::TypeDescriptor;
use hdf5::{Dataset, File as Hdf5File, Group, LinkType};
use hdf5_sys::h5::HADDR_UNDEF;
use hdf5_sys::h5d::{
    H5D_fill_value_t, H5Dget_chunk_info_by_coord, H5Dget_num_chunks, H5Dwrite_chunk,
};
use hdf5_sys::h5p::{H5Pfill_value_defined, H5Pget_fill_value, H5P_DEFAULT};
use hdf5_sys::h5t::H5Tencode;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::{ByteRange, SubsetLabError};

const MAX_METADATA_DATASET_BYTES: u64 = 1024 * 1024;
const MAX_METADATA_TOTAL_BYTES: u64 = 4 * 1024 * 1024;
const MAX_REQUIRED_CHUNKS: usize = 4_096;
const MAX_STORED_CHUNK_BYTES: u64 = 64 * 1024 * 1024;
const MAX_TOTAL_STORED_CHUNK_BYTES: u64 = 512 * 1024 * 1024;
const MAX_HDF5_IDENTITY_BYTES: usize = 1024 * 1024;

pub const DEFAULT_METADATA_PREFIX_BYTES: u64 = 128 * 1024 * 1024;

pub const SYNTHETIC_RSLC_METADATA_ALLOWLIST: [&str; 7] = [
    "/science/LSAR/RSLC/swaths/frequencyA/listOfPolarizations",
    "/science/LSAR/RSLC/swaths/frequencyB/listOfPolarizations",
    "/science/LSAR/identification/boundingPolygon",
    "/science/LSAR/identification/granuleId",
    "/science/LSAR/identification/listOfFrequencies",
    "/science/LSAR/identification/missionId",
    "/science/LSAR/identification/productType",
];

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Window2D {
    pub row_start: usize,
    pub row_end: usize,
    pub column_start: usize,
    pub column_end: usize,
}

impl Window2D {
    pub fn new(
        row_start: usize,
        row_end: usize,
        column_start: usize,
        column_end: usize,
        shape: [usize; 2],
    ) -> Result<Self, SubsetLabError> {
        if row_start >= row_end
            || column_start >= column_end
            || row_end > shape[0]
            || column_end > shape[1]
        {
            return Err(SubsetLabError::InvalidWindow);
        }
        Ok(Self {
            row_start,
            row_end,
            column_start,
            column_end,
        })
    }

    pub fn shape(self) -> [usize; 2] {
        [
            self.row_end - self.row_start,
            self.column_end - self.column_start,
        ]
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum SupportedFilter {
    Shuffle,
    Deflate(u8),
    Fletcher32,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ChunkRecord {
    pub logical_offset: [usize; 2],
    pub filter_mask: u32,
    pub file_range: Option<ByteRange>,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FillValueDefinition {
    #[default]
    Unspecified,
    Undefined,
    Default,
    UserDefined,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ChunkLayout {
    pub source_path: PathBuf,
    pub dataset_path: String,
    pub source_len: u64,
    pub dataset_shape: [usize; 2],
    pub chunk_shape: [usize; 2],
    /// Human-readable descriptor retained for diagnostics, not datatype identity.
    pub datatype: String,
    /// SHA-256 of the exact `H5Tencode` representation, including byte order.
    #[serde(default)]
    pub datatype_sha256: String,
    /// HDF5's fill-definition state from `H5Pfill_value_defined`.
    #[serde(default)]
    pub fill_value_definition: FillValueDefinition,
    /// Lowercase hex of `H5Pget_fill_value` bytes in the exact dataset datatype.
    pub fill_value: Option<String>,
    pub filters: Vec<SupportedFilter>,
    pub window: Window2D,
    pub chunks: Vec<ChunkRecord>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RawChunkPayload {
    pub logical_offset: [usize; 2],
    pub filter_mask: u32,
    pub bytes: Vec<u8>,
    pub stored_sha256: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PrefixDiscovery {
    pub prefix_bytes: u64,
    pub source_len: u64,
    pub attempts: u32,
    pub layout: ChunkLayout,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct MetadataCopyRecord {
    pub path: String,
    pub element_count: usize,
    pub logical_bytes: u64,
    pub storage_bytes: u64,
    pub datatype: String,
}

#[derive(Debug, PartialEq, Eq)]
struct FillValueIdentity {
    definition: FillValueDefinition,
    bytes: Option<Vec<u8>>,
}

pub fn discover_chunk_layout(
    source_path: &Path,
    dataset_path: &str,
    requested: Window2D,
) -> Result<ChunkLayout, SubsetLabError> {
    let source_len = fs::metadata(source_path)?.len();
    if source_len == 0 || !valid_absolute_hdf5_path(dataset_path) {
        return Err(SubsetLabError::DatasetLayoutUnsupported);
    }
    let file = Hdf5File::open(source_path)?;
    if !path_uses_only_hard_links(&file, dataset_path)? {
        return Err(SubsetLabError::ExternalOrVirtualStorage);
    }
    let dataset = file.dataset(dataset_path)?;
    let dcpl = dataset.dcpl()?;
    if dataset.filename() != file.filename()
        || dcpl.get_layout()? == Layout::Virtual
        || !dcpl.get_external()?.is_empty()
    {
        return Err(SubsetLabError::ExternalOrVirtualStorage);
    }
    let shape = fixed_rank_two_shape(&dataset)?;
    let requested = Window2D::new(
        requested.row_start,
        requested.row_end,
        requested.column_start,
        requested.column_end,
        shape,
    )?;
    let chunk = dcpl
        .get_chunk()?
        .filter(|chunk| chunk.len() == 2 && chunk.iter().all(|value| *value > 0))
        .ok_or(SubsetLabError::DatasetLayoutUnsupported)?;
    let chunk_shape = [chunk[0], chunk[1]];
    let filters = supported_filters(&dataset)?;
    let descriptor = dataset.dtype()?.to_descriptor()?;
    if !fixed_width_metadata_type(&descriptor) {
        return Err(SubsetLabError::DatasetLayoutUnsupported);
    }
    let datatype = format!("{descriptor:?}");
    let datatype_sha256 = datatype_sha256(&dataset)?;
    let fill_identity = fill_value_identity(&dataset)?;
    let fill_value = fill_identity.bytes.as_deref().map(hex_bytes);

    let required_offsets = required_chunk_offsets(requested, chunk_shape)?;
    let chunks = required_offsets
        .into_iter()
        .map(|offset| chunk_record_by_coordinate(&dataset, offset, source_len))
        .collect::<Result<Vec<_>, _>>()?;
    if chunks.is_empty() {
        return Err(SubsetLabError::ChunkMetadataInvalid);
    }
    Ok(ChunkLayout {
        source_path: source_path.to_path_buf(),
        dataset_path: dataset_path.to_owned(),
        source_len,
        dataset_shape: shape,
        chunk_shape,
        datatype,
        datatype_sha256,
        fill_value_definition: fill_identity.definition,
        fill_value,
        filters,
        window: requested,
        chunks,
    })
}

pub fn read_raw_chunk_payloads(
    layout: &ChunkLayout,
) -> Result<Vec<RawChunkPayload>, SubsetLabError> {
    validate_layout_against_source(layout)?;
    let ranges = layout
        .chunks
        .iter()
        .filter_map(|chunk| chunk.file_range.map(|range| (chunk, range)))
        .collect::<Vec<_>>();
    validate_stored_chunk_budget(ranges.iter().map(|(_, range)| range.len()))?;

    let mut source = File::open(&layout.source_path)?;
    let mut payloads = Vec::with_capacity(ranges.len());
    for (chunk, range) in ranges {
        let size = usize::try_from(range.len()).map_err(|_| SubsetLabError::ChunkBudgetExceeded)?;
        let mut bytes = vec![0_u8; size];
        source.seek(SeekFrom::Start(range.start()))?;
        source.read_exact(&mut bytes)?;
        let stored_sha256 = hex_sha256(&bytes);
        payloads.push(RawChunkPayload {
            logical_offset: chunk.logical_offset,
            filter_mask: chunk.filter_mask,
            bytes,
            stored_sha256,
        });
    }
    Ok(payloads)
}

pub fn write_raw_chunk_payloads(
    destination: &Dataset,
    layout: &ChunkLayout,
    payloads: &[RawChunkPayload],
    source_origin: [usize; 2],
) -> Result<(), SubsetLabError> {
    if !source_origin[0].is_multiple_of(layout.chunk_shape[0])
        || !source_origin[1].is_multiple_of(layout.chunk_shape[1])
    {
        return Err(SubsetLabError::UnalignedDirectChunkOrigin);
    }
    let destination_shape = fixed_rank_two_shape(destination)?;
    let source_dataset = validate_layout_against_source(layout)?;
    let source_fill_identity = fill_value_identity(&source_dataset)?;
    if datatype_sha256(destination)? != layout.datatype_sha256
        || fill_value_identity(destination)? != source_fill_identity
        || destination.dcpl()?.get_chunk()?.as_deref() != Some(layout.chunk_shape.as_slice())
        || supported_filters(&source_dataset)? != layout.filters
        || supported_filters(destination)? != layout.filters
    {
        return Err(SubsetLabError::DatasetLayoutUnsupported);
    }
    let expected = layout
        .chunks
        .iter()
        .filter(|chunk| chunk.file_range.is_some())
        .map(|chunk| (chunk.logical_offset, chunk))
        .collect::<BTreeMap<_, _>>();
    if payloads.len() != expected.len() {
        return Err(SubsetLabError::ChunkMetadataInvalid);
    }
    validate_stored_chunk_budget(payloads.iter().map(|payload| payload.bytes.len() as u64))?;

    if allocated_chunk_count(destination)? != 0 {
        return Err(SubsetLabError::DestinationNotEmpty);
    }
    let mut destination_offsets = BTreeMap::new();
    for chunk in &layout.chunks {
        let destination_offset = [
            chunk.logical_offset[0]
                .checked_sub(source_origin[0])
                .ok_or(SubsetLabError::UnalignedDirectChunkOrigin)?,
            chunk.logical_offset[1]
                .checked_sub(source_origin[1])
                .ok_or(SubsetLabError::UnalignedDirectChunkOrigin)?,
        ];
        if destination_offset[0] >= destination_shape[0]
            || destination_offset[1] >= destination_shape[1]
            || destination_offsets
                .insert(
                    chunk.logical_offset,
                    [destination_offset[0] as u64, destination_offset[1] as u64],
                )
                .is_some()
        {
            return Err(SubsetLabError::UnalignedDirectChunkOrigin);
        }
    }

    let mut validated = BTreeSet::new();
    let mut prepared = Vec::with_capacity(payloads.len());
    for payload in payloads {
        let Some(source_record) = expected.get(&payload.logical_offset) else {
            return Err(SubsetLabError::ChunkMetadataInvalid);
        };
        let source_range = source_record
            .file_range
            .ok_or(SubsetLabError::ChunkMetadataInvalid)?;
        if payload.filter_mask != source_record.filter_mask
            || payload.bytes.len()
                != usize::try_from(source_range.len())
                    .map_err(|_| SubsetLabError::ChunkMetadataInvalid)?
            || hex_sha256(&payload.bytes) != payload.stored_sha256
            || !validated.insert(payload.logical_offset)
        {
            return Err(SubsetLabError::ChunkMetadataInvalid);
        }
        let offset = *destination_offsets
            .get(&payload.logical_offset)
            .ok_or(SubsetLabError::ChunkMetadataInvalid)?;
        prepared.push((payload, offset));
    }

    for (payload, offset) in prepared {
        let status = hdf5::sync::sync(|| unsafe {
            H5Dwrite_chunk(
                destination.id(),
                H5P_DEFAULT,
                payload.filter_mask,
                offset.as_ptr(),
                payload.bytes.len(),
                payload.bytes.as_ptr().cast(),
            )
        });
        if status < 0 {
            return Err(SubsetLabError::Hdf5(
                "H5Dwrite_chunk rejected a bounded payload".to_owned(),
            ));
        }
    }
    destination.flush()?;
    Ok(())
}

fn validate_layout_against_source(layout: &ChunkLayout) -> Result<Dataset, SubsetLabError> {
    if fs::metadata(&layout.source_path)?.len() != layout.source_len {
        return Err(SubsetLabError::ChunkMetadataInvalid);
    }
    let source_file = Hdf5File::open(&layout.source_path)?;
    if !path_uses_only_hard_links(&source_file, &layout.dataset_path)? {
        return Err(SubsetLabError::ChunkMetadataInvalid);
    }
    let source_dataset = source_file.dataset(&layout.dataset_path)?;
    let dcpl = source_dataset.dcpl()?;
    let descriptor = source_dataset.dtype()?.to_descriptor()?;
    let datatype_sha256 = datatype_sha256(&source_dataset)?;
    let fill_identity = fill_value_identity(&source_dataset)?;
    let fill_value = fill_identity.bytes.as_deref().map(hex_bytes);
    if source_dataset.filename() != source_file.filename()
        || dcpl.get_layout()? == Layout::Virtual
        || !dcpl.get_external()?.is_empty()
        || fixed_rank_two_shape(&source_dataset)? != layout.dataset_shape
        || dcpl.get_chunk()?.as_deref() != Some(layout.chunk_shape.as_slice())
        || format!("{descriptor:?}") != layout.datatype
        || datatype_sha256 != layout.datatype_sha256
        || fill_identity.definition != layout.fill_value_definition
        || fill_value != layout.fill_value
        || supported_filters(&source_dataset)? != layout.filters
    {
        return Err(SubsetLabError::ChunkMetadataInvalid);
    }
    let window = Window2D::new(
        layout.window.row_start,
        layout.window.row_end,
        layout.window.column_start,
        layout.window.column_end,
        layout.dataset_shape,
    )?;
    let required_offsets = required_chunk_offsets(window, layout.chunk_shape)?;
    if layout.chunks.len() != required_offsets.len()
        || layout
            .chunks
            .iter()
            .zip(&required_offsets)
            .any(|(chunk, required)| chunk.logical_offset != *required)
    {
        return Err(SubsetLabError::ChunkMetadataInvalid);
    }
    for expected in &layout.chunks {
        if chunk_record_by_coordinate(&source_dataset, expected.logical_offset, layout.source_len)?
            != *expected
        {
            return Err(SubsetLabError::ChunkMetadataInvalid);
        }
    }
    Ok(source_dataset)
}

pub fn discover_chunk_layout_with_prefix(
    source_path: &Path,
    scratch_path: &Path,
    dataset_path: &str,
    requested: Window2D,
    initial_prefix_bytes: u64,
    maximum_prefix_bytes: u64,
) -> Result<PrefixDiscovery, SubsetLabError> {
    let source_len = fs::metadata(source_path)?.len();
    if source_path == scratch_path
        || source_len == 0
        || initial_prefix_bytes == 0
        || maximum_prefix_bytes < initial_prefix_bytes
    {
        return Err(SubsetLabError::MetadataPrefixInsufficient);
    }
    validate_scratch_path(source_path, scratch_path)?;
    let maximum = maximum_prefix_bytes.min(source_len);
    let mut prefix = initial_prefix_bytes.min(source_len);
    let mut attempts = 0_u32;
    loop {
        attempts = attempts
            .checked_add(1)
            .ok_or(SubsetLabError::MetadataPrefixInsufficient)?;
        materialize_sparse_prefix(source_path, scratch_path, source_len, prefix)?;
        match discover_chunk_layout(scratch_path, dataset_path, requested) {
            Ok(mut layout) => {
                layout.source_path = source_path.to_path_buf();
                return Ok(PrefixDiscovery {
                    prefix_bytes: prefix,
                    source_len,
                    attempts,
                    layout,
                });
            }
            Err(
                error @ (SubsetLabError::InvalidWindow
                | SubsetLabError::DatasetLayoutUnsupported
                | SubsetLabError::ExternalOrVirtualStorage
                | SubsetLabError::FilterUnsupported
                | SubsetLabError::ChunkBudgetExceeded),
            ) => return Err(error),
            Err(_) => {}
        }
        if prefix >= maximum {
            return Err(SubsetLabError::MetadataPrefixInsufficient);
        }
        prefix = prefix.saturating_mul(2).min(maximum);
    }
}

fn validate_scratch_path(source_path: &Path, scratch_path: &Path) -> Result<(), SubsetLabError> {
    let Ok(scratch_link_metadata) = fs::symlink_metadata(scratch_path) else {
        return Ok(());
    };
    if scratch_link_metadata.file_type().is_symlink()
        || paths_refer_to_same_file(source_path, scratch_path)?
    {
        return Err(SubsetLabError::ScratchPathUnsafe);
    }
    Ok(())
}

#[cfg(windows)]
fn paths_refer_to_same_file(left: &Path, right: &Path) -> Result<bool, SubsetLabError> {
    use std::os::windows::io::AsRawHandle;

    use windows_sys::Win32::Storage::FileSystem::{
        GetFileInformationByHandle, BY_HANDLE_FILE_INFORMATION,
    };

    fn identity(path: &Path) -> Result<(u32, u64), SubsetLabError> {
        let file = File::open(path)?;
        let mut information = BY_HANDLE_FILE_INFORMATION::default();
        // SAFETY: the handle remains owned by `file` for the call and the output
        // points to an initialized structure of the exact Windows API type.
        let status = unsafe { GetFileInformationByHandle(file.as_raw_handle(), &mut information) };
        if status == 0 {
            return Err(std::io::Error::last_os_error().into());
        }
        let file_index =
            (u64::from(information.nFileIndexHigh) << 32) | u64::from(information.nFileIndexLow);
        Ok((information.dwVolumeSerialNumber, file_index))
    }

    Ok(identity(left)? == identity(right)?)
}

#[cfg(unix)]
fn paths_refer_to_same_file(left: &Path, right: &Path) -> Result<bool, SubsetLabError> {
    use std::os::unix::fs::MetadataExt;

    let left = fs::metadata(left)?;
    let right = fs::metadata(right)?;
    Ok(left.dev() == right.dev() && left.ino() == right.ino())
}

#[cfg(not(any(unix, windows)))]
fn paths_refer_to_same_file(left: &Path, right: &Path) -> Result<bool, SubsetLabError> {
    Ok(fs::canonicalize(left)? == fs::canonicalize(right)?)
}

pub fn copy_allowlisted_datasets(
    source: &Hdf5File,
    destination: &Hdf5File,
    paths: &[&str],
) -> Result<Vec<MetadataCopyRecord>, SubsetLabError> {
    if paths.is_empty() || paths.len() > SYNTHETIC_RSLC_METADATA_ALLOWLIST.len() {
        return Err(SubsetLabError::MetadataPathInvalid);
    }
    let mut unique = BTreeSet::new();
    for path in paths {
        if !valid_absolute_hdf5_path(path)
            || !SYNTHETIC_RSLC_METADATA_ALLOWLIST.contains(path)
            || !unique.insert(*path)
        {
            return Err(SubsetLabError::MetadataPathInvalid);
        }
    }
    let mut validated = Vec::with_capacity(paths.len());
    let mut total_bytes = 0_u64;
    for path in unique {
        let dataset = source
            .dataset(path)
            .map_err(|_| SubsetLabError::MetadataPathInvalid)?;
        if !path_uses_only_hard_links(source, path)? {
            return Err(SubsetLabError::MetadataPathInvalid);
        }
        let dcpl = dataset.dcpl()?;
        let space = dataset.space()?;
        let extents = space.extents()?;
        if dataset.filename() != source.filename()
            || dcpl.get_layout()? == Layout::Virtual
            || !dcpl.get_external()?.is_empty()
            || !dcpl.get_filters()?.is_empty()
            || extents
                .maxdims()
                .iter()
                .zip(extents.dims())
                .any(|(maximum, current)| *maximum != Some(current))
        {
            return Err(SubsetLabError::MetadataPathInvalid);
        }
        if !dataset.attr_names()?.is_empty() {
            return Err(SubsetLabError::MetadataAttributeUnsupported);
        }
        let descriptor = dataset.dtype()?.to_descriptor()?;
        if !fixed_width_metadata_type(&descriptor) {
            return Err(SubsetLabError::MetadataTypeUnsupported);
        }
        let element_count = dataset.size();
        let logical_bytes = u64::try_from(element_count)
            .ok()
            .and_then(|elements| {
                u64::try_from(descriptor.size())
                    .ok()
                    .and_then(|size| elements.checked_mul(size))
            })
            .ok_or(SubsetLabError::MetadataBudgetExceeded)?;
        let storage_bytes = dataset.storage_size();
        let budgeted_bytes = logical_bytes.max(storage_bytes);
        total_bytes = total_bytes
            .checked_add(budgeted_bytes)
            .ok_or(SubsetLabError::MetadataBudgetExceeded)?;
        if budgeted_bytes > MAX_METADATA_DATASET_BYTES || total_bytes > MAX_METADATA_TOTAL_BYTES {
            return Err(SubsetLabError::MetadataBudgetExceeded);
        }
        let (parent_path, name) = path
            .rsplit_once('/')
            .filter(|(_, name)| !name.is_empty())
            .ok_or(SubsetLabError::MetadataPathInvalid)?;
        if destination.link_exists(path) {
            return Err(SubsetLabError::MetadataPathInvalid);
        }
        validated.push((
            parent_path.to_owned(),
            name.to_owned(),
            dataset,
            MetadataCopyRecord {
                path: path.to_owned(),
                element_count,
                logical_bytes,
                storage_bytes,
                datatype: format!("{descriptor:?}"),
            },
        ));
    }
    let copy_policy = ObjectCopy::build().copy_without_attr(true).finish()?;
    let mut records = Vec::with_capacity(validated.len());
    for (parent_path, name, dataset, record) in validated {
        let parent = ensure_group(destination, &parent_path)?;
        dataset.copy_to_with_props(&parent, &name, Some(&copy_policy), None)?;
        records.push(record);
    }
    records.sort_by(|left, right| left.path.cmp(&right.path));
    Ok(records)
}

fn datatype_sha256(dataset: &Dataset) -> Result<String, SubsetLabError> {
    let datatype = dataset.dtype()?;
    let mut encoded_len = 0_usize;
    let size_status = hdf5::sync::sync(|| unsafe {
        H5Tencode(datatype.id(), std::ptr::null_mut(), &mut encoded_len)
    });
    if size_status < 0 || encoded_len == 0 || encoded_len > MAX_HDF5_IDENTITY_BYTES {
        return Err(SubsetLabError::DatasetLayoutUnsupported);
    }

    let mut encoded = vec![0_u8; encoded_len];
    let mut written_len = encoded_len;
    let encode_status = hdf5::sync::sync(|| unsafe {
        H5Tencode(datatype.id(), encoded.as_mut_ptr().cast(), &mut written_len)
    });
    if encode_status < 0 || written_len != encoded_len {
        return Err(SubsetLabError::DatasetLayoutUnsupported);
    }
    Ok(hex_sha256(&encoded))
}

fn fill_value_identity(dataset: &Dataset) -> Result<FillValueIdentity, SubsetLabError> {
    use H5D_fill_value_t::{
        H5D_FILL_VALUE_DEFAULT, H5D_FILL_VALUE_ERROR, H5D_FILL_VALUE_UNDEFINED,
        H5D_FILL_VALUE_USER_DEFINED,
    };

    let dcpl = dataset.dcpl()?;
    let mut raw_definition = H5D_FILL_VALUE_ERROR;
    let definition_status =
        hdf5::sync::sync(|| unsafe { H5Pfill_value_defined(dcpl.id(), &mut raw_definition) });
    if definition_status < 0 {
        return Err(SubsetLabError::Hdf5(
            "H5Pfill_value_defined rejected a dataset creation property list".to_owned(),
        ));
    }
    let definition = match raw_definition {
        H5D_FILL_VALUE_UNDEFINED => FillValueDefinition::Undefined,
        H5D_FILL_VALUE_DEFAULT => FillValueDefinition::Default,
        H5D_FILL_VALUE_USER_DEFINED => FillValueDefinition::UserDefined,
        H5D_FILL_VALUE_ERROR => {
            return Err(SubsetLabError::Hdf5(
                "H5Pfill_value_defined returned an error state".to_owned(),
            ));
        }
    };
    if definition == FillValueDefinition::Undefined {
        return Ok(FillValueIdentity {
            definition,
            bytes: None,
        });
    }

    let datatype = dataset.dtype()?;
    let value_len = datatype.size();
    if value_len == 0 || value_len > MAX_HDF5_IDENTITY_BYTES {
        return Err(SubsetLabError::DatasetLayoutUnsupported);
    }
    let mut bytes = vec![0_u8; value_len];
    let value_status = hdf5::sync::sync(|| unsafe {
        H5Pget_fill_value(dcpl.id(), datatype.id(), bytes.as_mut_ptr().cast())
    });
    if value_status < 0 {
        return Err(SubsetLabError::Hdf5(
            "H5Pget_fill_value could not return exact dataset-type bytes".to_owned(),
        ));
    }
    Ok(FillValueIdentity {
        definition,
        bytes: Some(bytes),
    })
}

fn fixed_width_metadata_type(descriptor: &TypeDescriptor) -> bool {
    match descriptor {
        TypeDescriptor::Compound(compound) => compound
            .fields
            .iter()
            .all(|field| fixed_width_metadata_type(&field.ty)),
        TypeDescriptor::FixedArray(element, _) => fixed_width_metadata_type(element),
        TypeDescriptor::VarLenArray(_)
        | TypeDescriptor::VarLenAscii
        | TypeDescriptor::VarLenUnicode
        | TypeDescriptor::Reference(_) => false,
        _ => true,
    }
}

fn fixed_rank_two_shape(dataset: &Dataset) -> Result<[usize; 2], SubsetLabError> {
    let extents = dataset.space()?.extents()?;
    let shape = extents.dims();
    let maxdims = extents.maxdims();
    if shape.len() != 2
        || maxdims.len() != 2
        || shape.contains(&0)
        || maxdims
            .iter()
            .zip(&shape)
            .any(|(maximum, current)| *maximum != Some(*current))
    {
        return Err(SubsetLabError::DatasetLayoutUnsupported);
    }
    Ok([shape[0], shape[1]])
}

fn supported_filters(dataset: &Dataset) -> Result<Vec<SupportedFilter>, SubsetLabError> {
    dataset
        .dcpl()?
        .get_filters()?
        .into_iter()
        .map(|filter| match filter {
            Filter::Shuffle => Ok(SupportedFilter::Shuffle),
            Filter::Deflate(level) if level <= 9 => Ok(SupportedFilter::Deflate(level)),
            Filter::Fletcher32 => Ok(SupportedFilter::Fletcher32),
            _ => Err(SubsetLabError::FilterUnsupported),
        })
        .collect()
}

fn required_chunk_offsets(
    window: Window2D,
    chunk: [usize; 2],
) -> Result<Vec<[usize; 2]>, SubsetLabError> {
    let first_row = (window.row_start / chunk[0]) * chunk[0];
    let first_column = (window.column_start / chunk[1]) * chunk[1];
    let row_count = (window.row_end - 1 - first_row) / chunk[0] + 1;
    let column_count = (window.column_end - 1 - first_column) / chunk[1] + 1;
    let count = row_count
        .checked_mul(column_count)
        .filter(|count| *count <= MAX_REQUIRED_CHUNKS)
        .ok_or(SubsetLabError::ChunkBudgetExceeded)?;
    let mut offsets = Vec::with_capacity(count);
    for row in (first_row..window.row_end).step_by(chunk[0]) {
        for column in (first_column..window.column_end).step_by(chunk[1]) {
            offsets.push([row, column]);
        }
    }
    Ok(offsets)
}

fn chunk_record_by_coordinate(
    dataset: &Dataset,
    logical_offset: [usize; 2],
    source_len: u64,
) -> Result<ChunkRecord, SubsetLabError> {
    let offset = [logical_offset[0] as u64, logical_offset[1] as u64];
    let mut filter_mask = 0_u32;
    let mut address = HADDR_UNDEF;
    let mut size = 0_u64;
    let status = hdf5::sync::sync(|| unsafe {
        H5Dget_chunk_info_by_coord(
            dataset.id(),
            offset.as_ptr(),
            &mut filter_mask,
            &mut address,
            &mut size,
        )
    });
    if status < 0 {
        return Err(SubsetLabError::ChunkMetadataInvalid);
    }
    let file_range = match (address == HADDR_UNDEF, size == 0) {
        (true, true) => None,
        (false, false) => {
            let end = address
                .checked_add(size)
                .ok_or(SubsetLabError::ChunkMetadataInvalid)?;
            Some(
                ByteRange::new(address, end, source_len)
                    .map_err(|_| SubsetLabError::ChunkMetadataInvalid)?,
            )
        }
        _ => return Err(SubsetLabError::ChunkMetadataInvalid),
    };
    Ok(ChunkRecord {
        logical_offset,
        filter_mask,
        file_range,
    })
}

fn validate_stored_chunk_budget(
    lengths: impl IntoIterator<Item = u64>,
) -> Result<(), SubsetLabError> {
    let mut total = 0_u64;
    for length in lengths {
        if length == 0 || length > MAX_STORED_CHUNK_BYTES {
            return Err(SubsetLabError::ChunkBudgetExceeded);
        }
        total = total
            .checked_add(length)
            .filter(|total| *total <= MAX_TOTAL_STORED_CHUNK_BYTES)
            .ok_or(SubsetLabError::ChunkBudgetExceeded)?;
    }
    Ok(())
}

fn allocated_chunk_count(dataset: &Dataset) -> Result<usize, SubsetLabError> {
    let space = dataset.space()?;
    let mut count = 0_u64;
    let status =
        hdf5::sync::sync(|| unsafe { H5Dget_num_chunks(dataset.id(), space.id(), &mut count) });
    if status < 0 {
        return Err(SubsetLabError::ChunkMetadataInvalid);
    }
    usize::try_from(count).map_err(|_| SubsetLabError::ChunkBudgetExceeded)
}

fn path_uses_only_hard_links(file: &Hdf5File, path: &str) -> Result<bool, SubsetLabError> {
    if !valid_absolute_hdf5_path(path) {
        return Ok(false);
    }
    let mut group = file.group("/")?;
    let components = path
        .split('/')
        .filter(|component| !component.is_empty())
        .collect::<Vec<_>>();
    for (index, component) in components.iter().enumerate() {
        let link_type = group.iter_visit_default(None, |_, name, info, found| {
            if name == *component {
                *found = Some(info.link_type);
                false
            } else {
                true
            }
        })?;
        if link_type != Some(LinkType::Hard) {
            return Ok(false);
        }
        if index + 1 < components.len() {
            group = group.group(component)?;
        }
    }
    Ok(!components.is_empty())
}

fn hex_sha256(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

fn hex_bytes(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";

    let mut output = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        output.push(HEX[usize::from(byte >> 4)] as char);
        output.push(HEX[usize::from(byte & 0x0f)] as char);
    }
    output
}

fn materialize_sparse_prefix(
    source_path: &Path,
    scratch_path: &Path,
    source_len: u64,
    prefix_len: u64,
) -> Result<(), SubsetLabError> {
    let parent = scratch_path
        .parent()
        .ok_or(SubsetLabError::MetadataPrefixInsufficient)?;
    fs::create_dir_all(parent)?;
    let mut source = File::open(source_path)?;
    let mut scratch = OpenOptions::new()
        .create(true)
        .truncate(true)
        .read(true)
        .write(true)
        .open(scratch_path)?;
    scratch.set_len(source_len)?;
    let mut remaining = prefix_len;
    let mut buffer = [0_u8; 64 * 1024];
    while remaining > 0 {
        let wanted = usize::try_from(remaining.min(buffer.len() as u64))
            .map_err(|_| SubsetLabError::MetadataPrefixInsufficient)?;
        source.read_exact(&mut buffer[..wanted])?;
        scratch.write_all(&buffer[..wanted])?;
        remaining -= wanted as u64;
    }
    scratch.flush()?;
    Ok(())
}

fn valid_absolute_hdf5_path(path: &str) -> bool {
    path.starts_with('/')
        && path.len() <= 2_048
        && !path.ends_with('/')
        && path
            .split('/')
            .skip(1)
            .all(|component| !component.is_empty() && component != "." && component != "..")
}

fn ensure_group(file: &Hdf5File, path: &str) -> Result<Group, SubsetLabError> {
    let mut group = file.group("/")?;
    for component in path.split('/').filter(|component| !component.is_empty()) {
        group = if group.link_exists(component) {
            group.group(component)?
        } else {
            group.create_group(component)?
        };
    }
    Ok(group)
}
