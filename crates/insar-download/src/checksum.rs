use std::fmt::Write as _;
use std::fs::File;
use std::io::{self, BufReader, Read};
use std::path::Path;

use sha2::Digest;
use thiserror::Error;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ChecksumAlgorithm {
    Md5,
    Sha1,
    Sha256,
    Sha512,
}

impl ChecksumAlgorithm {
    pub fn parse(value: &str) -> Result<Self, ChecksumError> {
        match value.trim().to_ascii_lowercase().as_str() {
            "md5" => Ok(Self::Md5),
            "sha1" => Ok(Self::Sha1),
            "sha256" => Ok(Self::Sha256),
            "sha512" => Ok(Self::Sha512),
            _ => Err(ChecksumError::UnsupportedAlgorithm),
        }
    }

    pub const fn name(self) -> &'static str {
        match self {
            Self::Md5 => "md5",
            Self::Sha1 => "sha1",
            Self::Sha256 => "sha256",
            Self::Sha512 => "sha512",
        }
    }

    const fn hex_length(self) -> usize {
        match self {
            Self::Md5 => 32,
            Self::Sha1 => 40,
            Self::Sha256 => 64,
            Self::Sha512 => 128,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ExpectedChecksum {
    algorithm: ChecksumAlgorithm,
    value: String,
}

impl ExpectedChecksum {
    pub fn new(algorithm: &str, value: &str) -> Result<Self, ChecksumError> {
        let algorithm = ChecksumAlgorithm::parse(algorithm)?;
        let value = value.trim().to_ascii_lowercase();
        if value.len() != algorithm.hex_length()
            || !value.bytes().all(|byte| byte.is_ascii_hexdigit())
        {
            return Err(ChecksumError::InvalidValue);
        }
        Ok(Self { algorithm, value })
    }

    pub const fn algorithm(&self) -> ChecksumAlgorithm {
        self.algorithm
    }

    pub fn value(&self) -> &str {
        &self.value
    }
}

#[derive(Debug, Error)]
pub enum ChecksumError {
    #[error("unsupported checksum algorithm")]
    UnsupportedAlgorithm,
    #[error("invalid checksum value")]
    InvalidValue,
    #[error("could not read checksum input")]
    Io(#[source] io::Error),
    #[error("checksum mismatch")]
    Mismatch,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ChecksumVerificationReport {
    pub bytes_read: u64,
}

pub fn verify_file_checksum(path: &Path, expected: &ExpectedChecksum) -> Result<(), ChecksumError> {
    verify_file_checksum_with_report(path, expected).map(|_| ())
}

pub fn verify_file_checksum_with_report(
    path: &Path,
    expected: &ExpectedChecksum,
) -> Result<ChecksumVerificationReport, ChecksumError> {
    let (actual, bytes_read) = match expected.algorithm {
        ChecksumAlgorithm::Md5 => digest_file::<md5::Md5>(path),
        ChecksumAlgorithm::Sha1 => digest_file::<sha1::Sha1>(path),
        ChecksumAlgorithm::Sha256 => digest_file::<sha2::Sha256>(path),
        ChecksumAlgorithm::Sha512 => digest_file::<sha2::Sha512>(path),
    }
    .map_err(ChecksumError::Io)?;
    if actual == expected.value {
        Ok(ChecksumVerificationReport { bytes_read })
    } else {
        Err(ChecksumError::Mismatch)
    }
}

pub(crate) enum ChecksumAccumulator {
    Md5(md5::Md5),
    Sha1(sha1::Sha1),
    Sha256(sha2::Sha256),
    Sha512(sha2::Sha512),
}

impl ChecksumAccumulator {
    pub(crate) fn new(algorithm: ChecksumAlgorithm) -> Self {
        match algorithm {
            ChecksumAlgorithm::Md5 => Self::Md5(md5::Md5::new()),
            ChecksumAlgorithm::Sha1 => Self::Sha1(sha1::Sha1::new()),
            ChecksumAlgorithm::Sha256 => Self::Sha256(sha2::Sha256::new()),
            ChecksumAlgorithm::Sha512 => Self::Sha512(sha2::Sha512::new()),
        }
    }

    pub(crate) fn update(&mut self, bytes: &[u8]) {
        match self {
            Self::Md5(digest) => Digest::update(digest, bytes),
            Self::Sha1(digest) => Digest::update(digest, bytes),
            Self::Sha256(digest) => Digest::update(digest, bytes),
            Self::Sha512(digest) => Digest::update(digest, bytes),
        }
    }

    pub(crate) fn finalize_hex(self) -> String {
        match self {
            Self::Md5(digest) => encode_hex(digest.finalize()),
            Self::Sha1(digest) => encode_hex(digest.finalize()),
            Self::Sha256(digest) => encode_hex(digest.finalize()),
            Self::Sha512(digest) => encode_hex(digest.finalize()),
        }
    }
}

pub(crate) struct Sha256Accumulator(sha2::Sha256);

impl Default for Sha256Accumulator {
    fn default() -> Self {
        Self(sha2::Sha256::new())
    }
}

impl Sha256Accumulator {
    pub(crate) fn update(&mut self, bytes: &[u8]) {
        Digest::update(&mut self.0, bytes);
    }

    pub(crate) fn snapshot_hex(&self) -> String {
        encode_hex(self.0.clone().finalize())
    }

    pub(crate) fn finalize_hex(self) -> String {
        encode_hex(self.0.finalize())
    }
}

fn digest_file<D: Digest>(path: &Path) -> io::Result<(String, u64)> {
    let file = File::open(path)?;
    let mut reader = BufReader::new(file);
    let mut digest = D::new();
    let mut buffer = [0_u8; 64 * 1024];
    let mut bytes_read = 0_u64;
    loop {
        let read = reader.read(&mut buffer)?;
        if read == 0 {
            break;
        }
        digest.update(&buffer[..read]);
        bytes_read = bytes_read
            .checked_add(read as u64)
            .ok_or_else(|| io::Error::other("checksum byte counter overflow"))?;
    }
    Ok((encode_hex(digest.finalize()), bytes_read))
}

fn encode_hex(bytes: impl AsRef<[u8]>) -> String {
    let bytes = bytes.as_ref();
    let mut encoded = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        write!(&mut encoded, "{byte:02x}").expect("writing hexadecimal to String cannot fail");
    }
    encoded
}

#[cfg(test)]
mod tests {
    use super::{ChecksumAccumulator, ChecksumAlgorithm, Sha256Accumulator};

    #[test]
    fn incremental_checksum_accumulators_match_known_digests() {
        let mut md5 = ChecksumAccumulator::new(ChecksumAlgorithm::Md5);
        md5.update(b"a");
        md5.update(b"bc");
        assert_eq!(md5.finalize_hex(), "900150983cd24fb0d6963f7d28e17f72");

        let mut sha256 = Sha256Accumulator::default();
        sha256.update(b"a");
        assert_eq!(
            sha256.snapshot_hex(),
            "ca978112ca1bbdcafac231b39a23dc4da786eff8147c4e72b9807785afee48bb"
        );
        sha256.update(b"bc");
        assert_eq!(
            sha256.finalize_hex(),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
    }
}
