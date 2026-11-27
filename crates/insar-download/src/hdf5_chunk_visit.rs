//! One pass over the stored chunks of a chunked HDF5 dataset (downloader
//! enhancement D1d). A lookup by index (`H5Dget_chunk_info`) walks the chunk
//! index from its start, so enumerating n chunks that way costs O(n²): tens of
//! seconds for a full-frame NISAR dataset during every planning revalidation,
//! writer entry and pre-promotion check (L4 2026-10-18: a pause took 28.6 s to
//! take effect). `H5Dchunk_iter` visits every stored chunk once, in the same
//! chunk index order.

/// One stored chunk as the chunk index records it.
pub(crate) struct StoredChunk<'a> {
    /// Logical offset of the chunk's first element, one entry per dimension.
    pub(crate) offset: &'a [u64],
    pub(crate) filter_mask: u32,
    pub(crate) address: u64,
    pub(crate) size: u64,
}

/// Why a visit did not see every stored chunk.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum ChunkVisitError<E> {
    /// The dataset is not chunked.
    NotChunked,
    /// HDF5 could not iterate the chunk index.
    Hdf5,
    /// `each` ended the visit with this error.
    Ended(E),
}

/// Calls `each` for every stored chunk of `dataset`, once and in chunk index
/// order, and returns how many chunks were visited. `each` ends the visit by
/// returning an error.
pub(crate) fn visit_stored_chunks<E>(
    dataset: &hdf5::Dataset,
    mut each: impl FnMut(StoredChunk<'_>) -> Result<(), E>,
) -> Result<u64, ChunkVisitError<E>> {
    if !dataset.is_chunked() {
        return Err(ChunkVisitError::NotChunked);
    }
    let mut ended = None;
    let mut visited = 0_u64;
    let status = dataset.chunks_visit(|info| {
        visited += 1;
        match each(StoredChunk {
            offset: info.offset,
            filter_mask: info.filter_mask,
            address: info.addr,
            size: info.size,
        }) {
            Ok(()) => 0,
            Err(error) => {
                ended = Some(error);
                1
            }
        }
    });
    if let Some(error) = ended {
        return Err(ChunkVisitError::Ended(error));
    }
    status.map_err(|_| ChunkVisitError::Hdf5)?;
    Ok(visited)
}

#[cfg(test)]
mod tests {
    use std::time::Instant;

    use super::{visit_stored_chunks, ChunkVisitError};

    type Chunk = (Vec<u64>, u32, u64, u64);

    /// The previous enumeration: one lookup by index per chunk.
    fn chunks_by_index(dataset: &hdf5::Dataset) -> Vec<Chunk> {
        (0..dataset.num_chunks().expect("chunked dataset"))
            .map(|index| {
                let info = dataset.chunk_info(index).expect("chunk by index");
                (info.offset, info.filter_mask, info.addr, info.size)
            })
            .collect()
    }

    fn chunks_by_visit(dataset: &hdf5::Dataset) -> Vec<Chunk> {
        let mut chunks = Vec::new();
        let visited = visit_stored_chunks(dataset, |chunk| {
            chunks.push((
                chunk.offset.to_vec(),
                chunk.filter_mask,
                chunk.address,
                chunk.size,
            ));
            Ok::<(), ()>(())
        })
        .expect("visit");
        assert_eq!(visited, chunks.len() as u64);
        chunks
    }

    fn file(name: &str) -> (std::path::PathBuf, hdf5::File) {
        let root =
            std::env::temp_dir().join(format!("insar-chunk-visit-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();
        let path = root.join("chunks.h5");
        let file = hdf5::File::create(crate::long_path::native_path(&path)).unwrap();
        (root, file)
    }

    /// The visit sees exactly the chunks a lookup by index sees, in the same
    /// order, for allocated and unallocated chunks alike; it ends when asked;
    /// a contiguous dataset is reported as not chunked.
    #[test]
    fn a_visit_matches_the_lookup_by_index_and_stops_on_request() {
        let _guard = crate::hdf5_plugin_isolation::HDF5_TEST_LOCK
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let (root, file) = file("equivalence");
        let dataset = file
            .new_dataset::<u16>()
            .shape([24, 20])
            .chunk([4, 5])
            .create("sparse")
            .unwrap();
        // Write only some chunks: unallocated chunks are not stored.
        dataset
            .write_slice(
                &ndarray::Array2::<u16>::from_elem((8, 10), 7),
                (4..12, 5..15),
            )
            .unwrap();
        dataset
            .write_slice(
                &ndarray::Array2::<u16>::from_elem((4, 5), 9),
                (20..24, 15..20),
            )
            .unwrap();
        let by_index = chunks_by_index(&dataset);
        assert_eq!(by_index.len(), 5, "four chunks of the block and one corner");
        assert_eq!(chunks_by_visit(&dataset), by_index);

        let mut seen = 0;
        let stopped = visit_stored_chunks(&dataset, |_| {
            seen += 1;
            if seen == 2 {
                Err("stop")
            } else {
                Ok(())
            }
        });
        assert_eq!(stopped, Err(ChunkVisitError::Ended("stop")));
        assert_eq!(seen, 2);

        let contiguous = file
            .new_dataset::<u16>()
            .shape([4, 4])
            .create("contiguous")
            .unwrap();
        assert_eq!(
            visit_stored_chunks(&contiguous, |_| Ok::<(), ()>(())),
            Err(ChunkVisitError::NotChunked)
        );
        drop(file);
        let _ = std::fs::remove_dir_all(root);
    }

    /// Timing evidence (D1d item 4a): enumerating the stored chunks of one
    /// dataset by index versus one visit.
    #[test]
    #[ignore = "timing evidence; run with --ignored --nocapture"]
    fn chunk_enumeration_timing_by_index_versus_visit() {
        let _guard = crate::hdf5_plugin_isolation::HDF5_TEST_LOCK
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let (root, file) = file("timing");
        let side = 128_usize;
        let dataset = file
            .new_dataset::<u8>()
            .shape([side * 2, side * 2])
            .chunk([2, 2])
            .create("dense")
            .unwrap();
        dataset
            .write(&ndarray::Array2::<u8>::from_elem((side * 2, side * 2), 1))
            .unwrap();
        let started = Instant::now();
        let by_index = chunks_by_index(&dataset);
        let index_elapsed = started.elapsed();
        let started = Instant::now();
        let by_visit = chunks_by_visit(&dataset);
        let visit_elapsed = started.elapsed();
        assert_eq!(by_visit, by_index);
        eprintln!(
            "CHUNK_ENUMERATION chunks={} by_index_ms={} visit_ms={} debug_assertions={}",
            by_index.len(),
            index_elapsed.as_millis(),
            visit_elapsed.as_millis(),
            cfg!(debug_assertions)
        );
        drop(file);
        let _ = std::fs::remove_dir_all(root);
    }
}
