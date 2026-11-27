//! A single pass over an existing file computing every digest a local reuse
//! decision needs (downloader D3). The read stops between blocks when the
//! item is paused or cancelled; nothing is persisted, so a later decision
//! starts over.

use std::fs::File;
use std::io::{ErrorKind, Read, Seek, SeekFrom};

use crate::checksum::{ChecksumAccumulator, ChecksumAlgorithm};

const BLOCK: usize = 1024 * 1024;

/// Why a digest was not computed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum DigestError {
    /// A pause or cancellation ended the read.
    Stopped,
    /// The file could not be read.
    Io,
}

/// Reads `file` once from its start and returns the lowercase hex digest of
/// each requested algorithm, in the requested order.
pub(crate) fn digest_file(
    file: &File,
    algorithms: &[ChecksumAlgorithm],
    stop: &dyn Fn() -> bool,
) -> Result<Vec<String>, DigestError> {
    let mut reader = file.try_clone().map_err(|_| DigestError::Io)?;
    reader
        .seek(SeekFrom::Start(0))
        .map_err(|_| DigestError::Io)?;
    let mut accumulators = algorithms
        .iter()
        .map(|algorithm| ChecksumAccumulator::new(*algorithm))
        .collect::<Vec<_>>();
    let mut buffer = vec![0_u8; BLOCK];
    loop {
        if stop() {
            return Err(DigestError::Stopped);
        }
        let count = match reader.read(&mut buffer) {
            Ok(count) => count,
            Err(error) if error.kind() == ErrorKind::Interrupted => continue,
            Err(_) => return Err(DigestError::Io),
        };
        if count == 0 {
            break;
        }
        for accumulator in &mut accumulators {
            accumulator.update(&buffer[..count]);
        }
    }
    Ok(accumulators
        .into_iter()
        .map(ChecksumAccumulator::finalize_hex)
        .collect())
}
