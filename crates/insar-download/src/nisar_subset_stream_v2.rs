//! Bounded streaming of NISAR V2 derived windows (Issue #8, slice 6).
//!
//! The V2 writer used to capture every selected science and mask window in
//! memory before writing and validating it, so the AOI size was bounded by a
//! fixed working set. This module streams each derived window instead: read a
//! source hyperslab, write it to the candidate, and in the validation pass
//! re-read the source and the candidate and compare them bit for bit.
//!
//! Tiles start on output chunk boundaries: a tile is a band of whole chunk
//! rows across the full window width, or, when one chunk row alone exceeds the
//! band budget, a run of whole chunks inside one chunk row. Every chunk is
//! therefore written by exactly one HDF5 call and chunks are visited in
//! row-major order, the same order as one whole-window write, so the
//! candidate bytes do not depend on the budget. The output volume formula is
//! shared by planning admission and the writer.

use std::path::Path;
#[cfg(test)]
use std::sync::atomic::{AtomicU64, Ordering};

use hdf5::{Dataset, H5Type};

use crate::nisar_planning_v2::NisarSubsetPhysicalExecutionPlanV2;

/// Upper bound of one streamed tile buffer (science or mask), except that a
/// single output chunk is never split.
pub(crate) const V2_STREAM_BAND_BUDGET_BYTES: usize = 32 * 1024 * 1024;
/// No streamed tile, not even one output chunk, may exceed this.
pub(crate) const V2_STREAM_TILE_LIMIT_BYTES: usize = 64 * 1024 * 1024;
/// Logical bytes of all derived windows of one artifact: the V1 output bound
/// (`MAX_OUTPUT_LOGICAL_BYTES`), now independent of the writer working set.
pub(crate) const V2_OUTPUT_LOGICAL_LIMIT_BYTES: u64 = 2 * 1024 * 1024 * 1024;
/// Free space kept beyond the computed requirement for HDF5 structure, the
/// sidecar and allocation rounding.
pub(crate) const V2_DISK_HEADROOM_BYTES: u64 = 64 * 1024 * 1024;

/// `u64::MAX` means no override.
#[cfg(test)]
static V2_AVAILABLE_DISK_OVERRIDE: AtomicU64 = AtomicU64::new(u64::MAX);
/// `0` means no override.
#[cfg(test)]
static V2_OUTPUT_LIMIT_OVERRIDE: AtomicU64 = AtomicU64::new(0);

#[cfg(test)]
type V2AfterTileHook = Box<dyn Fn(V2StreamPass, usize)>;

#[cfg(test)]
thread_local! {
    static V2_AFTER_TILE_HOOK: std::cell::RefCell<Option<V2AfterTileHook>> =
        std::cell::RefCell::new(None);
    static V2_VERIFY_READ_FAULT: std::cell::Cell<Option<usize>> =
        const { std::cell::Cell::new(None) };
    // The band budget override and the tile observations belong to the test
    // thread that drives the writer: library tests run in parallel, and a
    // process-wide value let one test's writer and guard leak into another
    // test's observation (a 2 MiB budget reset mid-run, foreign tiles).
    static V2_STREAM_BUDGET_OVERRIDE: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
    static V2_STREAM_MAX_TILE_BYTES_OBSERVED: std::cell::Cell<usize> =
        const { std::cell::Cell::new(0) };
    static V2_STREAM_TILES_OBSERVED: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

/// Which streaming pass reached a tile boundary (test observation only).
#[cfg_attr(not(test), allow(dead_code))]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum V2StreamPass {
    Write,
    Verify,
}

/// Why a V2 output was refused before any candidate byte was written.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum V2OutputAdmissionRefusal {
    /// The derived windows exceed the output bound; more disk does not help.
    OutputLimit { required: u64, limit: u64 },
    /// The output volume lacks free space for the derived artifact.
    DiskSpace { required: u64, available: u64 },
}

/// The band budget in effect (a test may lower it).
pub(crate) fn v2_stream_band_budget() -> usize {
    #[cfg(test)]
    {
        let budget = V2_STREAM_BUDGET_OVERRIDE.with(std::cell::Cell::get);
        if budget > 0 {
            return budget;
        }
    }
    V2_STREAM_BAND_BUDGET_BYTES
}

/// One derived window of one polarization: the windows of one component and
/// ordinal (window index) share a validity mask and scales when their extents
/// match, as in the writer's derived layout.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct V2DerivedWindowExtent {
    pub(crate) component_id: u16,
    pub(crate) ordinal: usize,
    pub(crate) line_start: u64,
    pub(crate) line_end_exclusive: u64,
    pub(crate) sample_start: u64,
    pub(crate) sample_end_exclusive: u64,
}

/// Logical bytes of every derived dataset of one artifact: each science
/// window (8 bytes per complex sample), each distinct validity mask window
/// (1 byte per sample) and each distinct zero-Doppler-time and slant-range
/// scale (8 bytes per value). `None` on an inverted extent or overflow.
pub(crate) fn v2_derived_output_logical_bytes_of(
    windows: impl IntoIterator<Item = V2DerivedWindowExtent>,
) -> Option<u64> {
    let mut masks = std::collections::BTreeSet::new();
    let mut lines = std::collections::BTreeSet::new();
    let mut samples = std::collections::BTreeSet::new();
    let mut total = 0_u64;
    for window in windows {
        let rows = window.line_end_exclusive.checked_sub(window.line_start)?;
        let cols = window
            .sample_end_exclusive
            .checked_sub(window.sample_start)?;
        let cells = rows.checked_mul(cols)?;
        total = total.checked_add(cells.checked_mul(8)?)?;
        let group = (window.component_id, window.ordinal);
        if masks.insert((
            group,
            window.line_start,
            window.line_end_exclusive,
            window.sample_start,
            window.sample_end_exclusive,
        )) {
            total = total.checked_add(cells)?;
        }
        if lines.insert((group, window.line_start, window.line_end_exclusive)) {
            total = total.checked_add(rows.checked_mul(8)?)?;
        }
        if samples.insert((group, window.sample_start, window.sample_end_exclusive)) {
            total = total.checked_add(cols.checked_mul(8)?)?;
        }
    }
    Some(total)
}

/// The derived windows of a physical plan, in the writer's layout.
pub(crate) fn v2_plan_window_extents(
    plan: &NisarSubsetPhysicalExecutionPlanV2,
) -> Vec<V2DerivedWindowExtent> {
    plan.dataset_mappings()
        .iter()
        .flat_map(|mapping| {
            mapping
                .windows()
                .iter()
                .enumerate()
                .map(|(ordinal, window)| V2DerivedWindowExtent {
                    component_id: window.component_id(),
                    ordinal,
                    line_start: window.line_start(),
                    line_end_exclusive: window.line_end_exclusive(),
                    sample_start: window.sample_start(),
                    sample_end_exclusive: window.sample_end_exclusive(),
                })
        })
        .collect()
}

/// Logical bytes of every derived dataset of the plan's artifact.
pub(crate) fn v2_derived_output_logical_bytes(
    plan: &NisarSubsetPhysicalExecutionPlanV2,
) -> Option<u64> {
    v2_derived_output_logical_bytes_of(v2_plan_window_extents(plan))
}

/// The output bound in effect (a test may lower it).
pub(crate) fn v2_output_logical_limit() -> u64 {
    #[cfg(test)]
    {
        let limit = V2_OUTPUT_LIMIT_OVERRIDE.load(Ordering::SeqCst);
        if limit > 0 {
            return limit;
        }
    }
    V2_OUTPUT_LOGICAL_LIMIT_BYTES
}

/// Checks a derived output size against the shared bound; an unrepresentable
/// size is refused as the largest possible one.
pub(crate) fn v2_check_output_bytes(
    required: Option<u64>,
) -> Result<u64, V2OutputAdmissionRefusal> {
    let required = required.unwrap_or(u64::MAX);
    let limit = v2_output_logical_limit();
    if required > limit {
        return Err(V2OutputAdmissionRefusal::OutputLimit { required, limit });
    }
    Ok(required)
}

/// The shared output bound of planning admission and the writer entry.
pub(crate) fn v2_check_output_limit(
    plan: &NisarSubsetPhysicalExecutionPlanV2,
) -> Result<u64, V2OutputAdmissionRefusal> {
    v2_check_output_bytes(v2_derived_output_logical_bytes(plan))
}

/// Largest output chunk of the plan's science windows in bytes (a chunk is
/// clipped to its window, as the writer creates it).
pub(crate) fn v2_largest_output_chunk_bytes(plan: &NisarSubsetPhysicalExecutionPlanV2) -> u64 {
    plan.dataset_mappings()
        .iter()
        .flat_map(|mapping| {
            let chunk = mapping.chunk_shape();
            mapping.windows().iter().map(move |window| {
                let rows = window
                    .line_end_exclusive()
                    .saturating_sub(window.line_start())
                    .min(chunk[0]);
                let cols = window
                    .sample_end_exclusive()
                    .saturating_sub(window.sample_start())
                    .min(chunk[1]);
                rows.saturating_mul(cols).saturating_mul(8)
            })
        })
        .max()
        .unwrap_or(0)
}

/// Admission of a planned output (Issue #8, slice 6), shared by planning
/// before the Store freezes a plan and by the worker before any science
/// request of a frozen plan: the derived output must fit the shared bound and
/// the output volume must hold it together with `pending_transfer_bytes` of
/// Ranges still to transfer. Returns the derived output bytes.
pub(crate) fn v2_admit_planned_output(
    plan: &NisarSubsetPhysicalExecutionPlanV2,
    output_root: &Path,
    pending_transfer_bytes: u64,
) -> Result<u64, V2OutputAdmissionRefusal> {
    let output = v2_check_output_limit(plan)?;
    v2_check_disk_space(output_root, output.saturating_add(pending_transfer_bytes))?;
    Ok(output)
}

/// The admission refusal carried by a planning error, if any.
pub(crate) fn v2_output_refusal_of(
    error: crate::NisarSubsetPlanningErrorV2,
) -> Option<V2OutputAdmissionRefusal> {
    match error {
        crate::NisarSubsetPlanningErrorV2::OutputLimitExceeded { required, limit } => {
            Some(V2OutputAdmissionRefusal::OutputLimit { required, limit })
        }
        crate::NisarSubsetPlanningErrorV2::OutputDiskSpaceInsufficient {
            required,
            available,
        } => Some(V2OutputAdmissionRefusal::DiskSpace {
            required,
            available,
        }),
        _ => None,
    }
}

/// The planning error of a refused output (an explicit mapping: a `From`
/// impl would make `?` conversions into the planning error ambiguous).
pub(crate) fn v2_planning_error_of(
    refusal: V2OutputAdmissionRefusal,
) -> crate::NisarSubsetPlanningErrorV2 {
    match refusal {
        V2OutputAdmissionRefusal::OutputLimit { required, limit } => {
            crate::NisarSubsetPlanningErrorV2::OutputLimitExceeded { required, limit }
        }
        V2OutputAdmissionRefusal::DiskSpace {
            required,
            available,
        } => crate::NisarSubsetPlanningErrorV2::OutputDiskSpaceInsufficient {
            required,
            available,
        },
    }
}

/// Requires `required` bytes plus the headroom free on the volume of
/// `directory`. A volume whose free space cannot be queried is not refused:
/// the write itself still reports a real persistence failure.
pub(crate) fn v2_check_disk_space(
    directory: &Path,
    required: u64,
) -> Result<(), V2OutputAdmissionRefusal> {
    let required = required.saturating_add(V2_DISK_HEADROOM_BYTES);
    match available_disk_bytes(directory) {
        Some(available) if available < required => Err(V2OutputAdmissionRefusal::DiskSpace {
            required,
            available,
        }),
        _ => Ok(()),
    }
}

/// Free bytes available to this process on the volume of `directory`.
pub(crate) fn available_disk_bytes(directory: &Path) -> Option<u64> {
    #[cfg(test)]
    {
        let available = V2_AVAILABLE_DISK_OVERRIDE.load(Ordering::SeqCst);
        if available != u64::MAX {
            return Some(available);
        }
    }
    platform_available_disk_bytes(directory)
}

#[cfg(windows)]
fn platform_available_disk_bytes(directory: &Path) -> Option<u64> {
    use std::os::windows::ffi::OsStrExt;
    use windows_sys::Win32::Storage::FileSystem::GetDiskFreeSpaceExW;

    let wide = directory
        .as_os_str()
        .encode_wide()
        .chain(std::iter::once(0))
        .collect::<Vec<u16>>();
    let mut available = 0_u64;
    let mut total = 0_u64;
    let mut free = 0_u64;
    // SAFETY: `wide` is a NUL-terminated UTF-16 path that outlives the call
    // and the three out-pointers refer to live, writable u64 locals.
    let status =
        unsafe { GetDiskFreeSpaceExW(wide.as_ptr(), &mut available, &mut total, &mut free) };
    (status != 0).then_some(available)
}

#[cfg(unix)]
#[allow(clippy::useless_conversion)]
fn platform_available_disk_bytes(directory: &Path) -> Option<u64> {
    use std::os::unix::ffi::OsStrExt;

    let path = std::ffi::CString::new(directory.as_os_str().as_bytes()).ok()?;
    let mut facts = std::mem::MaybeUninit::<libc::statvfs>::uninit();
    // SAFETY: `path` is NUL-terminated and `facts` is writable storage for
    // exactly one statvfs record that is read only after success.
    let status = unsafe { libc::statvfs(path.as_ptr(), facts.as_mut_ptr()) };
    if status != 0 {
        return None;
    }
    // SAFETY: statvfs returned success, so the record is initialized.
    let facts = unsafe { facts.assume_init() };
    u64::from(facts.f_bavail).checked_mul(u64::from(facts.f_frsize))
}

#[cfg(not(any(windows, unix)))]
fn platform_available_disk_bytes(_directory: &Path) -> Option<u64> {
    None
}

/// One streamed rectangle in output coordinates (half-open).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct V2StreamTile {
    pub(crate) row_start: usize,
    pub(crate) row_end: usize,
    pub(crate) col_start: usize,
    pub(crate) col_end: usize,
}

#[cfg(test)]
impl V2StreamTile {
    fn cells(&self) -> usize {
        (self.row_end - self.row_start) * (self.col_end - self.col_start)
    }
}

/// The tiles of one derived window of `shape` whose output chunks are
/// `chunk` clipped to the window, as the writer creates them. `None` for an
/// empty or unrepresentable layout or when one output chunk alone exceeds
/// [`V2_STREAM_TILE_LIMIT_BYTES`].
pub(crate) fn v2_stream_tiles(
    shape: [u64; 2],
    chunk: [u64; 2],
    element_size: usize,
    budget: usize,
) -> Option<Vec<V2StreamTile>> {
    let rows = usize::try_from(shape[0]).ok()?;
    let cols = usize::try_from(shape[1]).ok()?;
    if rows == 0 || cols == 0 || element_size == 0 || chunk.contains(&0) {
        return None;
    }
    let chunk_rows = rows.min(usize::try_from(chunk[0]).unwrap_or(rows));
    let chunk_cols = cols.min(usize::try_from(chunk[1]).unwrap_or(cols));
    let chunk_bytes = chunk_rows
        .checked_mul(chunk_cols)?
        .checked_mul(element_size)?;
    if chunk_bytes > V2_STREAM_TILE_LIMIT_BYTES {
        return None;
    }
    let chunk_row_bytes = chunk_rows.checked_mul(cols)?.checked_mul(element_size)?;
    let mut tiles = Vec::new();
    if chunk_row_bytes <= budget {
        let band_chunk_rows = (budget / chunk_row_bytes)
            .max(1)
            .min(rows.div_ceil(chunk_rows));
        let band_rows = band_chunk_rows.checked_mul(chunk_rows)?;
        let mut row_start = 0;
        while row_start < rows {
            let row_end = row_start.saturating_add(band_rows).min(rows);
            tiles.push(V2StreamTile {
                row_start,
                row_end,
                col_start: 0,
                col_end: cols,
            });
            row_start = row_end;
        }
    } else {
        // One chunk row is wider than the budget: split it into runs of whole
        // chunks. Visiting the runs left to right keeps row-major chunk order.
        let tile_chunk_cols = (budget / chunk_bytes).max(1).min(cols.div_ceil(chunk_cols));
        let tile_cols = tile_chunk_cols.checked_mul(chunk_cols)?;
        let mut row_start = 0;
        while row_start < rows {
            let row_end = row_start.saturating_add(chunk_rows).min(rows);
            let mut col_start = 0;
            while col_start < cols {
                let col_end = col_start.saturating_add(tile_cols).min(cols);
                tiles.push(V2StreamTile {
                    row_start,
                    row_end,
                    col_start,
                    col_end,
                });
                col_start = col_end;
            }
            row_start = row_end;
        }
    }
    Some(tiles)
}

/// Largest tile buffer of `tiles` in bytes.
#[cfg(test)]
pub(crate) fn v2_largest_tile_bytes(tiles: &[V2StreamTile], element_size: usize) -> usize {
    tiles
        .iter()
        .map(|tile| tile.cells().saturating_mul(element_size))
        .max()
        .unwrap_or(0)
}

/// An element whose values compare by their exact bit patterns, so a NaN
/// sample equals the same NaN and `0.0` differs from `-0.0`.
pub(crate) trait V2StreamElement: H5Type + Copy {
    fn same_bits(&self, other: &Self) -> bool;
}

impl V2StreamElement for u8 {
    fn same_bits(&self, other: &Self) -> bool {
        self == other
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum V2StreamError {
    Cancelled,
    SourceRead,
    CandidateWrite,
    CandidateRead,
    Mismatch,
}

fn source_selection(
    origin: [u64; 2],
    tile: &V2StreamTile,
) -> Option<(std::ops::Range<usize>, std::ops::Range<usize>)> {
    let line = usize::try_from(origin[0]).ok()?;
    let sample = usize::try_from(origin[1]).ok()?;
    Some((
        line.checked_add(tile.row_start)?..line.checked_add(tile.row_end)?,
        sample.checked_add(tile.col_start)?..sample.checked_add(tile.col_end)?,
    ))
}

fn read_source_tile<T: V2StreamElement>(
    source: &Dataset,
    origin: [u64; 2],
    tile: &V2StreamTile,
) -> Result<ndarray::Array2<T>, V2StreamError> {
    let selection = source_selection(origin, tile).ok_or(V2StreamError::SourceRead)?;
    let values = source
        .read_slice_2d::<T, _>(selection)
        .map_err(|_| V2StreamError::SourceRead)?;
    if values.dim() != (tile.row_end - tile.row_start, tile.col_end - tile.col_start) {
        return Err(V2StreamError::SourceRead);
    }
    Ok(values)
}

fn observe_tile(pass: V2StreamPass, index: usize, bytes: usize) {
    #[cfg(test)]
    {
        V2_STREAM_MAX_TILE_BYTES_OBSERVED.with(|largest| largest.set(largest.get().max(bytes)));
        V2_STREAM_TILES_OBSERVED.with(|tiles| tiles.set(tiles.get().saturating_add(1)));
        V2_AFTER_TILE_HOOK.with(|hook| {
            if let Some(hook) = hook.borrow().as_ref() {
                hook(pass, index);
            }
        });
    }
    #[cfg(not(test))]
    let _ = (pass, index, bytes);
}

/// Copies one derived window tile by tile from `source` (window origin
/// `origin`) into the whole `candidate` dataset. Cancellation is observed
/// before every read and after every write.
pub(crate) fn v2_stream_copy<T: V2StreamElement>(
    source: &Dataset,
    origin: [u64; 2],
    candidate: &Dataset,
    tiles: &[V2StreamTile],
    cancelled: &(dyn Fn() -> bool + Sync),
) -> Result<(), V2StreamError> {
    for (index, tile) in tiles.iter().enumerate() {
        if cancelled() {
            return Err(V2StreamError::Cancelled);
        }
        let values = read_source_tile::<T>(source, origin, tile)?;
        candidate
            .write_slice(
                &values,
                (tile.row_start..tile.row_end, tile.col_start..tile.col_end),
            )
            .map_err(|_| V2StreamError::CandidateWrite)?;
        observe_tile(
            V2StreamPass::Write,
            index,
            values.len().saturating_mul(std::mem::size_of::<T>()),
        );
        if cancelled() {
            return Err(V2StreamError::Cancelled);
        }
    }
    Ok(())
}

/// Re-reads every tile of the source window and the candidate dataset and
/// requires bit-identical samples.
pub(crate) fn v2_stream_verify<T: V2StreamElement>(
    source: &Dataset,
    origin: [u64; 2],
    candidate: &Dataset,
    tiles: &[V2StreamTile],
    cancelled: &(dyn Fn() -> bool + Sync),
) -> Result<(), V2StreamError> {
    for (index, tile) in tiles.iter().enumerate() {
        if cancelled() {
            return Err(V2StreamError::Cancelled);
        }
        let expected = read_source_tile::<T>(source, origin, tile)?;
        #[cfg(test)]
        if V2_VERIFY_READ_FAULT.with(|fault| {
            let armed = fault.get() == Some(index);
            if armed {
                fault.set(None);
            }
            armed
        }) {
            return Err(V2StreamError::CandidateRead);
        }
        let actual = candidate
            .read_slice_2d::<T, _>((tile.row_start..tile.row_end, tile.col_start..tile.col_end))
            .map_err(|_| V2StreamError::CandidateRead)?;
        if actual.dim() != expected.dim()
            || actual
                .iter()
                .zip(expected.iter())
                .any(|(left, right)| !left.same_bits(right))
        {
            return Err(V2StreamError::Mismatch);
        }
        observe_tile(
            V2StreamPass::Verify,
            index,
            expected.len().saturating_mul(std::mem::size_of::<T>()),
        );
    }
    if cancelled() {
        return Err(V2StreamError::Cancelled);
    }
    Ok(())
}

/// Test-only: the next validation pass on this thread fails to read the
/// candidate at tile `index`, once, as a transient read error would.
#[cfg(test)]
pub(crate) struct V2VerifyReadFaultForTest(());

#[cfg(test)]
impl V2VerifyReadFaultForTest {
    pub(crate) fn arm(index: usize) -> Self {
        V2_VERIFY_READ_FAULT.with(|fault| fault.set(Some(index)));
        Self(())
    }
}

#[cfg(test)]
impl Drop for V2VerifyReadFaultForTest {
    fn drop(&mut self) {
        V2_VERIFY_READ_FAULT.with(|fault| fault.set(None));
    }
}

/// Test-only override of the band budget on this thread, restored on drop.
#[cfg(test)]
pub(crate) struct V2StreamBudgetForTest(());

#[cfg(test)]
impl V2StreamBudgetForTest {
    pub(crate) fn set(budget: usize) -> Self {
        assert!(budget > 0);
        V2_STREAM_BUDGET_OVERRIDE.with(|value| value.set(budget));
        Self(())
    }
}

#[cfg(test)]
impl Drop for V2StreamBudgetForTest {
    fn drop(&mut self) {
        V2_STREAM_BUDGET_OVERRIDE.with(|value| value.set(0));
    }
}

/// Test-only override of the shared output bound, restored on drop.
#[cfg(test)]
pub(crate) struct V2OutputLimitForTest(());

#[cfg(test)]
impl V2OutputLimitForTest {
    pub(crate) fn set(limit: u64) -> Self {
        assert!(limit > 0);
        V2_OUTPUT_LIMIT_OVERRIDE.store(limit, Ordering::SeqCst);
        Self(())
    }
}

#[cfg(test)]
impl Drop for V2OutputLimitForTest {
    fn drop(&mut self) {
        V2_OUTPUT_LIMIT_OVERRIDE.store(0, Ordering::SeqCst);
    }
}

/// Test-only override of the free space reported for every volume.
#[cfg(test)]
pub(crate) struct V2AvailableDiskForTest(());

#[cfg(test)]
impl V2AvailableDiskForTest {
    pub(crate) fn set(available: u64) -> Self {
        assert!(available != u64::MAX);
        V2_AVAILABLE_DISK_OVERRIDE.store(available, Ordering::SeqCst);
        Self(())
    }
}

#[cfg(test)]
impl Drop for V2AvailableDiskForTest {
    fn drop(&mut self) {
        V2_AVAILABLE_DISK_OVERRIDE.store(u64::MAX, Ordering::SeqCst);
    }
}

#[cfg(test)]
pub(crate) fn reset_v2_stream_observation_for_test() {
    V2_STREAM_MAX_TILE_BYTES_OBSERVED.with(|largest| largest.set(0));
    V2_STREAM_TILES_OBSERVED.with(|tiles| tiles.set(0));
}

/// (largest tile buffer in bytes, tiles streamed) on this thread since the
/// last reset.
#[cfg(test)]
pub(crate) fn v2_stream_observation_for_test() -> (usize, usize) {
    (
        V2_STREAM_MAX_TILE_BYTES_OBSERVED.with(std::cell::Cell::get),
        V2_STREAM_TILES_OBSERVED.with(std::cell::Cell::get),
    )
}

/// Runs `hook` after every streamed tile on this thread until the guard drops.
#[cfg(test)]
pub(crate) struct V2AfterTileHookForTest(());

#[cfg(test)]
impl V2AfterTileHookForTest {
    pub(crate) fn set(hook: impl Fn(V2StreamPass, usize) + 'static) -> Self {
        V2_AFTER_TILE_HOOK.with(|slot| *slot.borrow_mut() = Some(Box::new(hook)));
        Self(())
    }
}

#[cfg(test)]
impl Drop for V2AfterTileHookForTest {
    fn drop(&mut self) {
        V2_AFTER_TILE_HOOK.with(|slot| *slot.borrow_mut() = None);
    }
}

#[cfg(test)]
#[path = "nisar_subset_stream_v2_tests.rs"]
mod tests;
