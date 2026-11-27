//! Local-only feasibility code for NISAR remote HDF5 experiments.
//!
//! This crate is intentionally not a dependency of any production crate. It
//! cannot register a provider, resolver, task, command, Store, or UI route.

mod hdf5_layout;
mod http_range;
mod range_plan;
mod sparse_grid;

pub use hdf5_layout::{
    copy_allowlisted_datasets, discover_chunk_layout, discover_chunk_layout_with_prefix,
    read_raw_chunk_payloads, write_raw_chunk_payloads, ChunkLayout, ChunkRecord,
    FillValueDefinition, MetadataCopyRecord, PrefixDiscovery, RawChunkPayload, SupportedFilter,
    Window2D, DEFAULT_METADATA_PREFIX_BYTES, SYNTHETIC_RSLC_METADATA_ALLOWLIST,
};
pub use http_range::{
    fetch_local_range_plan, ByteLedger, LocalRangeSource, RangeFailureKind, RangeTransferFailure,
    RangeTransferLimits, RangeWorkspace, VerifiedRangeSet,
};
pub use range_plan::{ByteRange, MergePolicy, RangePlan, ZARR_V3_SHARD_128_TAIL_INDEX_BYTES};
pub use sparse_grid::{
    admit_sparse_grid_selection, sparse_grid_polygon_cells_v1, validate_reference_window,
    SelectedSparseCell, SparseCellIntersection, SparseGeoGrid, SparseGridAdmission,
    SparseGridComponent, SparseGridError, SparseGridReferenceWindow, SparseGridSelection,
    SPARSE_GRID_CELL_MASK_SEMANTICS, SPARSE_GRID_MAX_AOI_GEOJSON_BYTES,
    SPARSE_GRID_MAX_CANDIDATE_REFERENCE_CHUNK_AREA_RATIO, SPARSE_GRID_POLYGON_CELLS_V1,
};

use thiserror::Error;

#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub enum SubsetLabError {
    #[error("invalid half-open range")]
    InvalidRange,
    #[error("range plan exceeds its frozen budget")]
    RangeBudgetExceeded,
    #[error("invalid two-dimensional window")]
    InvalidWindow,
    #[error("dataset is not a fixed rank-two chunked array")]
    DatasetLayoutUnsupported,
    #[error("dataset uses external or virtual storage")]
    ExternalOrVirtualStorage,
    #[error("dataset filter is outside the lab allowlist")]
    FilterUnsupported,
    #[error("chunk metadata is missing, inconsistent, or outside the source file")]
    ChunkMetadataInvalid,
    #[error("chunk window or stored payloads exceed the frozen lab budget")]
    ChunkBudgetExceeded,
    #[error("raw chunk destination must be empty before reconstruction")]
    DestinationNotEmpty,
    #[error("direct chunk relocation requires a chunk-aligned origin")]
    UnalignedDirectChunkOrigin,
    #[error("the bounded metadata prefix was insufficient")]
    MetadataPrefixInsufficient,
    #[error("metadata scratch path aliases the source or uses a symbolic link")]
    ScratchPathUnsafe,
    #[error("metadata path is not in the exact dataset allowlist")]
    MetadataPathInvalid,
    #[error("metadata datatype is variable length or reference-bearing")]
    MetadataTypeUnsupported,
    #[error("metadata attributes are outside the exact lab copy policy")]
    MetadataAttributeUnsupported,
    #[error("metadata copy exceeds its frozen byte budget")]
    MetadataBudgetExceeded,
    #[error("local HDF5 operation failed: {0}")]
    Hdf5(String),
    #[error("local file operation failed: {0}")]
    Io(String),
}

impl From<hdf5::Error> for SubsetLabError {
    fn from(error: hdf5::Error) -> Self {
        Self::Hdf5(error.to_string())
    }
}

impl From<std::io::Error> for SubsetLabError {
    fn from(error: std::io::Error) -> Self {
        Self::Io(error.to_string())
    }
}
