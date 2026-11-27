//! Timing evidence for the downloader enhancement D1d performance items. The
//! tests are ignored: they measure, they do not assert a speed, and they are
//! run on demand with `--ignored --nocapture` to record numbers.

use std::time::Instant;

use sha2::{Digest, Sha256};

/// D1d item 4b: SHA-256 throughput of the build under test. A planning
/// revalidation digests the planning material (195 MB per S-band item in the
/// L4 run); in a debug build an unoptimised sha2 dominates that time.
#[test]
#[ignore = "timing evidence; run with --ignored --nocapture"]
fn sha256_throughput_of_this_build() {
    let block = vec![0x5a_u8; 64 * 1024];
    let total_bytes: usize = 256 * 1024 * 1024;
    let started = Instant::now();
    let mut digest = Sha256::new();
    for _ in 0..total_bytes / block.len() {
        digest.update(&block);
    }
    let value = digest.finalize();
    let elapsed = started.elapsed();
    let mebibytes = total_bytes as f64 / (1024.0 * 1024.0);
    eprintln!(
        "SHA256_THROUGHPUT bytes={total_bytes} elapsed_ms={} mib_per_s={:.1} debug_assertions={} digest_prefix={:02x}{:02x}",
        elapsed.as_millis(),
        mebibytes / elapsed.as_secs_f64(),
        cfg!(debug_assertions),
        value[0],
        value[1],
    );
}
