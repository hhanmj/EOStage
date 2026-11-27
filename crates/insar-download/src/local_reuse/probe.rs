//! Bounded format checks of an existing file (downloader D3). A check reads
//! only the structures that prove the file is not truncated or garbled: a ZIP
//! end record and central directory, an HDF5 superblock and end address, TIFF
//! strip or tile ranges, TAR member headers, the closing element of a precise
//! orbit file. It never decompresses or reads the payload.

use std::fs::File;
use std::io::{self, Read, Seek, SeekFrom};

/// The formats a local reuse decision can check.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ProbeFormat {
    Zip,
    Hdf5,
    Tiff,
    Tar,
    PreciseOrbit,
    MetadataText,
    Xml,
    Json,
    Unsupported,
}

impl ProbeFormat {
    /// The check that applies to a planned file name.
    pub(crate) fn for_file_name(file_name: &str) -> Self {
        let lower = file_name.to_ascii_lowercase();
        if lower.ends_with(".zip") {
            Self::Zip
        } else if lower.ends_with(".h5") || lower.ends_with(".hdf5") || lower.ends_with(".he5") {
            Self::Hdf5
        } else if lower.ends_with(".tif") || lower.ends_with(".tiff") {
            Self::Tiff
        } else if lower.ends_with(".tar") {
            Self::Tar
        } else if lower.ends_with(".eof") {
            Self::PreciseOrbit
        } else if lower.ends_with("_mtl.txt") || lower.ends_with("_ang.txt") {
            Self::MetadataText
        } else if lower.ends_with(".xml") {
            Self::Xml
        } else if lower.ends_with(".json") {
            Self::Json
        } else {
            Self::Unsupported
        }
    }
}

/// The result of a format check.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ProbeOutcome {
    /// The structures that prove completeness were read and are consistent.
    Readable,
    /// The file is truncated, garbled or of another format.
    Unreadable,
    /// No check exists for this format.
    Unsupported,
    /// The check would read more than its budget (a huge ZIP directory).
    OverBudget,
}

const KIB: u64 = 1024;
const MIB: u64 = 1024 * KIB;
const ZIP_DIRECTORY_BUDGET: u64 = 32 * MIB;
const TIFF_ARRAY_BUDGET: u64 = 16 * MIB;
const TEXT_BUDGET: u64 = 16 * MIB;
const ORBIT_HEAD: u64 = 64 * KIB;
const ORBIT_TAIL: u64 = 4 * KIB;
const TAR_MEMBERS: usize = 10_000;
const TIFF_IFDS: usize = 64;

/// Checks `file` (of `size` bytes) as `format`. `product` is the planned
/// product name without extension; a Sentinel-1 SAFE ZIP must contain its
/// manifest under that name.
pub(crate) fn probe_file(
    file: &File,
    size: u64,
    format: ProbeFormat,
    product: Option<&str>,
) -> io::Result<ProbeOutcome> {
    if size == 0 {
        return Ok(ProbeOutcome::Unreadable);
    }
    let mut reader = file.try_clone()?;
    match format {
        ProbeFormat::Zip => probe_zip(&mut reader, size, product),
        ProbeFormat::Hdf5 => probe_hdf5(&mut reader, size),
        ProbeFormat::Tiff => probe_tiff(&mut reader, size),
        ProbeFormat::Tar => probe_tar(&mut reader, size),
        ProbeFormat::PreciseOrbit => probe_orbit(&mut reader, size),
        ProbeFormat::MetadataText => probe_metadata_text(&mut reader, size),
        ProbeFormat::Xml => probe_xml(&mut reader, size),
        ProbeFormat::Json => probe_json(&mut reader, size),
        ProbeFormat::Unsupported => Ok(ProbeOutcome::Unsupported),
    }
}

fn read_at(reader: &mut File, offset: u64, length: usize) -> io::Result<Vec<u8>> {
    reader.seek(SeekFrom::Start(offset))?;
    let mut buffer = vec![0_u8; length];
    reader.read_exact(&mut buffer)?;
    Ok(buffer)
}

fn u16_le(bytes: &[u8], at: usize) -> u16 {
    u16::from_le_bytes([bytes[at], bytes[at + 1]])
}

fn u32_le(bytes: &[u8], at: usize) -> u32 {
    u32::from_le_bytes([bytes[at], bytes[at + 1], bytes[at + 2], bytes[at + 3]])
}

fn u64_le(bytes: &[u8], at: usize) -> u64 {
    let mut value = [0_u8; 8];
    value.copy_from_slice(&bytes[at..at + 8]);
    u64::from_le_bytes(value)
}

// ---------------------------------------------------------------- ZIP

const ZIP_EOCD: u32 = 0x0605_4b50;
const ZIP64_LOCATOR: u32 = 0x0706_4b50;
const ZIP64_EOCD: u32 = 0x0606_4b50;
const ZIP_CENTRAL: u32 = 0x0201_4b50;
const ZIP_LOCAL: u32 = 0x0403_4b50;

fn probe_zip(reader: &mut File, size: u64, product: Option<&str>) -> io::Result<ProbeOutcome> {
    if size < 22 {
        return Ok(ProbeOutcome::Unreadable);
    }
    let tail_length = size.min(22 + 65_535);
    let tail = read_at(reader, size - tail_length, tail_length as usize)?;
    // The end record whose comment ends exactly at the end of the file.
    let Some(position) = (0..=tail.len() - 22).rev().find(|&index| {
        u32_le(&tail, index) == ZIP_EOCD
            && usize::from(u16_le(&tail, index + 20)) == tail.len() - index - 22
    }) else {
        return Ok(ProbeOutcome::Unreadable);
    };
    let end_record = size - tail_length + position as u64;
    let record = &tail[position..position + 22];
    if u16_le(record, 4) != 0 || u16_le(record, 6) != 0 {
        return Ok(ProbeOutcome::Unreadable);
    }
    let mut entries = u64::from(u16_le(record, 10));
    let mut directory_size = u64::from(u32_le(record, 12));
    let mut directory_offset = u64::from(u32_le(record, 16));
    let mut directory_limit = end_record;
    if entries == 0xFFFF || directory_size == 0xFFFF_FFFF || directory_offset == 0xFFFF_FFFF {
        if end_record < 20 {
            return Ok(ProbeOutcome::Unreadable);
        }
        let locator = read_at(reader, end_record - 20, 20)?;
        if u32_le(&locator, 0) != ZIP64_LOCATOR {
            return Ok(ProbeOutcome::Unreadable);
        }
        let zip64_offset = u64_le(&locator, 8);
        if zip64_offset
            .checked_add(56)
            .is_none_or(|end| end > end_record - 20)
        {
            return Ok(ProbeOutcome::Unreadable);
        }
        let zip64 = read_at(reader, zip64_offset, 56)?;
        if u32_le(&zip64, 0) != ZIP64_EOCD {
            return Ok(ProbeOutcome::Unreadable);
        }
        entries = u64_le(&zip64, 32);
        directory_size = u64_le(&zip64, 40);
        directory_offset = u64_le(&zip64, 48);
        directory_limit = zip64_offset;
    }
    if entries == 0
        || directory_offset
            .checked_add(directory_size)
            .is_none_or(|end| end > directory_limit)
    {
        return Ok(ProbeOutcome::Unreadable);
    }
    if directory_size > ZIP_DIRECTORY_BUDGET {
        return Ok(ProbeOutcome::OverBudget);
    }
    let directory = read_at(reader, directory_offset, directory_size as usize)?;
    let manifest = product.and_then(|product| {
        let upper = product.to_ascii_uppercase();
        upper
            .starts_with("S1")
            .then(|| format!("{product}.SAFE/manifest.safe"))
    });
    let mut manifest_found = manifest.is_none();
    let mut first_local_header = None;
    let mut position = 0_usize;
    for _ in 0..entries {
        if position + 46 > directory.len() || u32_le(&directory, position) != ZIP_CENTRAL {
            return Ok(ProbeOutcome::Unreadable);
        }
        let name_length = usize::from(u16_le(&directory, position + 28));
        let extra_length = usize::from(u16_le(&directory, position + 30));
        let comment_length = usize::from(u16_le(&directory, position + 32));
        let local_header = u32_le(&directory, position + 42);
        let name_start = position + 46;
        let next = name_start + name_length + extra_length + comment_length;
        if next > directory.len() {
            return Ok(ProbeOutcome::Unreadable);
        }
        if first_local_header.is_none() {
            first_local_header = Some(local_header);
        }
        if let Some(expected) = &manifest {
            if !manifest_found
                && directory[name_start..name_start + name_length]
                    .eq_ignore_ascii_case(expected.as_bytes())
            {
                manifest_found = true;
            }
        }
        position = next;
    }
    if position != directory.len() || !manifest_found {
        return Ok(ProbeOutcome::Unreadable);
    }
    if let Some(offset) = first_local_header.filter(|offset| *offset != 0xFFFF_FFFF) {
        let offset = u64::from(offset);
        if offset + 4 > directory_offset {
            return Ok(ProbeOutcome::Unreadable);
        }
        if u32_le(&read_at(reader, offset, 4)?, 0) != ZIP_LOCAL {
            return Ok(ProbeOutcome::Unreadable);
        }
    }
    Ok(ProbeOutcome::Readable)
}

// ---------------------------------------------------------------- HDF5

const HDF5_SIGNATURE: [u8; 8] = [0x89, b'H', b'D', b'F', b'\r', b'\n', 0x1a, b'\n'];

fn read_sized(bytes: &[u8], at: usize, width: usize) -> Option<u64> {
    let slice = bytes.get(at..at + width)?;
    let mut value = [0_u8; 8];
    value[..width].copy_from_slice(slice);
    Some(u64::from_le_bytes(value))
}

fn probe_hdf5(reader: &mut File, size: u64) -> io::Result<ProbeOutcome> {
    let mut offset = 0_u64;
    loop {
        if offset + 8 > size {
            return Ok(ProbeOutcome::Unreadable);
        }
        if read_at(reader, offset, 8)? == HDF5_SIGNATURE {
            break;
        }
        offset = if offset == 0 { 512 } else { offset * 2 };
    }
    let available = (size - offset).min(128) as usize;
    let block = read_at(reader, offset, available)?;
    if block.len() < 16 {
        return Ok(ProbeOutcome::Unreadable);
    }
    let undefined = |value: u64, width: usize| value == u64::MAX >> (64 - 8 * width as u32);
    let (base, end) = match block[8] {
        version @ (0 | 1) => {
            let width = usize::from(block[13]);
            if !matches!(width, 2 | 4 | 8) || !matches!(block[14], 2 | 4 | 8) {
                return Ok(ProbeOutcome::Unreadable);
            }
            let base_at = if version == 0 { 24 } else { 28 };
            let (Some(base), Some(end)) = (
                read_sized(&block, base_at, width),
                read_sized(&block, base_at + 2 * width, width),
            ) else {
                return Ok(ProbeOutcome::Unreadable);
            };
            (if undefined(base, width) { offset } else { base }, end)
        }
        2 | 3 => {
            let width = usize::from(block[9]);
            if !matches!(width, 2 | 4 | 8) || !matches!(block[10], 2 | 4 | 8) {
                return Ok(ProbeOutcome::Unreadable);
            }
            let checksum_at = 12 + 4 * width;
            let (Some(base), Some(end), Some(stored)) = (
                read_sized(&block, 12, width),
                read_sized(&block, 12 + 2 * width, width),
                block
                    .get(checksum_at..checksum_at + 4)
                    .map(|bytes| u32_le(bytes, 0)),
            ) else {
                return Ok(ProbeOutcome::Unreadable);
            };
            if lookup3(&block[..checksum_at], 0) != stored {
                return Ok(ProbeOutcome::Unreadable);
            }
            (if undefined(base, width) { offset } else { base }, end)
        }
        _ => return Ok(ProbeOutcome::Unreadable),
    };
    if base.checked_add(end).is_none_or(|limit| limit > size) || end == 0 {
        return Ok(ProbeOutcome::Unreadable);
    }
    Ok(ProbeOutcome::Readable)
}

/// Bob Jenkins' lookup3 hashlittle, byte-wise, as HDF5 checksums metadata.
pub(crate) fn lookup3(data: &[u8], initial: u32) -> u32 {
    fn rot(value: u32, bits: u32) -> u32 {
        value.rotate_left(bits)
    }
    let mut a = 0xdead_beef_u32
        .wrapping_add(data.len() as u32)
        .wrapping_add(initial);
    let mut b = a;
    let mut c = a;
    let mut rest = data;
    let word = |bytes: &[u8]| {
        u32::from(bytes[0])
            | (u32::from(bytes[1]) << 8)
            | (u32::from(bytes[2]) << 16)
            | (u32::from(bytes[3]) << 24)
    };
    while rest.len() > 12 {
        a = a.wrapping_add(word(&rest[0..4]));
        b = b.wrapping_add(word(&rest[4..8]));
        c = c.wrapping_add(word(&rest[8..12]));
        a = a.wrapping_sub(c);
        a ^= rot(c, 4);
        c = c.wrapping_add(b);
        b = b.wrapping_sub(a);
        b ^= rot(a, 6);
        a = a.wrapping_add(c);
        c = c.wrapping_sub(b);
        c ^= rot(b, 8);
        b = b.wrapping_add(a);
        a = a.wrapping_sub(c);
        a ^= rot(c, 16);
        c = c.wrapping_add(b);
        b = b.wrapping_sub(a);
        b ^= rot(a, 19);
        a = a.wrapping_add(c);
        c = c.wrapping_sub(b);
        c ^= rot(b, 4);
        b = b.wrapping_add(a);
        rest = &rest[12..];
    }
    if rest.is_empty() {
        return c;
    }
    let mut padded = [0_u8; 12];
    padded[..rest.len()].copy_from_slice(rest);
    a = a.wrapping_add(word(&padded[0..4]));
    b = b.wrapping_add(word(&padded[4..8]));
    c = c.wrapping_add(word(&padded[8..12]));
    c ^= b;
    c = c.wrapping_sub(rot(b, 14));
    a ^= c;
    a = a.wrapping_sub(rot(c, 11));
    b ^= a;
    b = b.wrapping_sub(rot(a, 25));
    c ^= b;
    c = c.wrapping_sub(rot(b, 16));
    a ^= c;
    a = a.wrapping_sub(rot(c, 4));
    b ^= a;
    b = b.wrapping_sub(rot(a, 14));
    c ^= b;
    c = c.wrapping_sub(rot(b, 24));
    c
}

// ---------------------------------------------------------------- TIFF

struct TiffReader {
    little: bool,
    big: bool,
}

impl TiffReader {
    fn u16(&self, bytes: &[u8], at: usize) -> u16 {
        let value = [bytes[at], bytes[at + 1]];
        if self.little {
            u16::from_le_bytes(value)
        } else {
            u16::from_be_bytes(value)
        }
    }
    fn u32(&self, bytes: &[u8], at: usize) -> u32 {
        let value = [bytes[at], bytes[at + 1], bytes[at + 2], bytes[at + 3]];
        if self.little {
            u32::from_le_bytes(value)
        } else {
            u32::from_be_bytes(value)
        }
    }
    fn u64(&self, bytes: &[u8], at: usize) -> u64 {
        let mut value = [0_u8; 8];
        value.copy_from_slice(&bytes[at..at + 8]);
        if self.little {
            u64::from_le_bytes(value)
        } else {
            u64::from_be_bytes(value)
        }
    }
    fn offset(&self, bytes: &[u8], at: usize) -> u64 {
        if self.big {
            self.u64(bytes, at)
        } else {
            u64::from(self.u32(bytes, at))
        }
    }
}

/// One IFD entry: type, count and the offset of its inline value or array.
struct TiffEntry {
    kind: u16,
    count: u64,
    value_at: u64,
    inline: Vec<u8>,
}

fn probe_tiff(reader: &mut File, size: u64) -> io::Result<ProbeOutcome> {
    if size < 8 {
        return Ok(ProbeOutcome::Unreadable);
    }
    let header = read_at(reader, 0, size.min(16) as usize)?;
    let little = match &header[0..2] {
        b"II" => true,
        b"MM" => false,
        _ => return Ok(ProbeOutcome::Unreadable),
    };
    let probe = TiffReader { little, big: false };
    let magic = probe.u16(&header, 2);
    let tiff = match magic {
        42 => TiffReader { little, big: false },
        43 if header.len() >= 16 && probe.u16(&header, 4) == 8 => TiffReader { little, big: true },
        _ => return Ok(ProbeOutcome::Unreadable),
    };
    let mut next = if tiff.big {
        tiff.u64(&header, 8)
    } else {
        u64::from(tiff.u32(&header, 4))
    };
    let mut budget = TIFF_ARRAY_BUDGET;
    let mut images = 0_usize;
    let (count_width, entry_width, value_width) = if tiff.big {
        (8_u64, 20_u64, 8_u64)
    } else {
        (2, 12, 4)
    };
    while next != 0 {
        images += 1;
        if images > TIFF_IFDS || next.checked_add(count_width).is_none_or(|end| end > size) {
            return Ok(ProbeOutcome::Unreadable);
        }
        let count_bytes = read_at(reader, next, count_width as usize)?;
        let entries = if tiff.big {
            tiff.u64(&count_bytes, 0)
        } else {
            u64::from(tiff.u16(&count_bytes, 0))
        };
        if entries == 0 || entries > 4096 {
            return Ok(ProbeOutcome::Unreadable);
        }
        let table_length = entries * entry_width + value_width;
        if (next + count_width)
            .checked_add(table_length)
            .is_none_or(|end| end > size)
        {
            return Ok(ProbeOutcome::Unreadable);
        }
        let table = read_at(reader, next + count_width, table_length as usize)?;
        let mut tags = std::collections::BTreeMap::new();
        for index in 0..entries as usize {
            let at = index * entry_width as usize;
            let tag = tiff.u16(&table, at);
            let kind = tiff.u16(&table, at + 2);
            let count = if tiff.big {
                tiff.u64(&table, at + 4)
            } else {
                u64::from(tiff.u32(&table, at + 4))
            };
            let value_offset = at + if tiff.big { 12 } else { 8 };
            let inline = table[value_offset..value_offset + value_width as usize].to_vec();
            tags.insert(
                tag,
                TiffEntry {
                    kind,
                    count,
                    value_at: tiff.offset(&table, value_offset),
                    inline,
                },
            );
        }
        let next_at = entries as usize * entry_width as usize;
        next = tiff.offset(&table, next_at);
        if !tags.contains_key(&256) || !tags.contains_key(&257) {
            return Ok(ProbeOutcome::Unreadable);
        }
        let (offsets_tag, counts_tag) = if tags.contains_key(&324) {
            (324, 325)
        } else {
            (273, 279)
        };
        let (Some(offsets), Some(counts)) = (tags.get(&offsets_tag), tags.get(&counts_tag)) else {
            return Ok(ProbeOutcome::Unreadable);
        };
        if offsets.count == 0 || offsets.count != counts.count {
            return Ok(ProbeOutcome::Unreadable);
        }
        let Some(offset_values) = tiff_values(reader, &tiff, offsets, size, &mut budget)? else {
            return Ok(ProbeOutcome::Unreadable);
        };
        let Some(count_values) = tiff_values(reader, &tiff, counts, size, &mut budget)? else {
            return Ok(ProbeOutcome::Unreadable);
        };
        if offset_values
            .iter()
            .zip(&count_values)
            .any(|(offset, count)| offset.checked_add(*count).is_none_or(|end| end > size))
        {
            return Ok(ProbeOutcome::Unreadable);
        }
    }
    if images == 0 {
        return Ok(ProbeOutcome::Unreadable);
    }
    Ok(ProbeOutcome::Readable)
}

/// The SHORT, LONG or LONG8 values of an entry, inline or from its array;
/// None when the entry is malformed or outside the file.
fn tiff_values(
    reader: &mut File,
    tiff: &TiffReader,
    entry: &TiffEntry,
    size: u64,
    budget: &mut u64,
) -> io::Result<Option<Vec<u64>>> {
    let width = match entry.kind {
        3 => 2_u64,
        4 => 4,
        16 => 8,
        _ => return Ok(None),
    };
    let Some(length) = entry.count.checked_mul(width) else {
        return Ok(None);
    };
    let inline_width = if tiff.big { 8 } else { 4 };
    let bytes = if length <= inline_width {
        entry.inline[..length as usize].to_vec()
    } else {
        if length > *budget
            || entry
                .value_at
                .checked_add(length)
                .is_none_or(|end| end > size)
        {
            return Ok(None);
        }
        *budget -= length;
        read_at(reader, entry.value_at, length as usize)?
    };
    Ok(Some(
        bytes
            .chunks_exact(width as usize)
            .map(|chunk| match width {
                2 => u64::from(tiff.u16(chunk, 0)),
                4 => u64::from(tiff.u32(chunk, 0)),
                _ => tiff.u64(chunk, 0),
            })
            .collect(),
    ))
}

// ---------------------------------------------------------------- TAR

fn tar_octal(field: &[u8]) -> Option<u64> {
    if field.first().is_some_and(|byte| byte & 0x80 != 0) {
        // GNU base-256: the remaining bytes are a big-endian number.
        let mut value = u64::from(field[0] & 0x7f);
        for byte in &field[1..] {
            value = value.checked_mul(256)?.checked_add(u64::from(*byte))?;
        }
        return Some(value);
    }
    let text = field
        .iter()
        .copied()
        .take_while(|byte| *byte != 0)
        .collect::<Vec<_>>();
    let text = std::str::from_utf8(&text).ok()?.trim();
    if text.is_empty() {
        return Some(0);
    }
    u64::from_str_radix(text, 8).ok()
}

fn probe_tar(reader: &mut File, size: u64) -> io::Result<ProbeOutcome> {
    let mut offset = 0_u64;
    let mut members = 0_usize;
    loop {
        if offset + 512 > size {
            return Ok(ProbeOutcome::Unreadable);
        }
        let header = read_at(reader, offset, 512)?;
        if header.iter().all(|byte| *byte == 0) {
            // The end: a second zero block must follow (record padding may add more).
            if offset + 1024 > size {
                return Ok(ProbeOutcome::Unreadable);
            }
            let second = read_at(reader, offset + 512, 512)?;
            return Ok(if second.iter().all(|byte| *byte == 0) && members > 0 {
                ProbeOutcome::Readable
            } else {
                ProbeOutcome::Unreadable
            });
        }
        members += 1;
        if members > TAR_MEMBERS || &header[257..262] != b"ustar" {
            return Ok(ProbeOutcome::Unreadable);
        }
        let Some(stored) = tar_octal(&header[148..156]) else {
            return Ok(ProbeOutcome::Unreadable);
        };
        let computed: u64 = header
            .iter()
            .enumerate()
            .map(|(index, byte)| {
                if (148..156).contains(&index) {
                    32
                } else {
                    u64::from(*byte)
                }
            })
            .sum();
        let Some(member_size) = tar_octal(&header[124..136]) else {
            return Ok(ProbeOutcome::Unreadable);
        };
        if stored != computed {
            return Ok(ProbeOutcome::Unreadable);
        }
        let padded = member_size.div_ceil(512) * 512;
        let Some(next) = offset
            .checked_add(512)
            .and_then(|value| value.checked_add(padded))
        else {
            return Ok(ProbeOutcome::Unreadable);
        };
        offset = next;
    }
}

// ---------------------------------------------------------------- text formats

fn probe_orbit(reader: &mut File, size: u64) -> io::Result<ProbeOutcome> {
    let head = read_at(reader, 0, size.min(ORBIT_HEAD) as usize)?;
    let head = String::from_utf8_lossy(&head);
    if !head.contains("<Earth_Explorer_File")
        || !head.contains("<File_Type>AUX_")
        || !head.contains("<Validity_Start>")
        || !head.contains("<Validity_Stop>")
    {
        return Ok(ProbeOutcome::Unreadable);
    }
    let tail_length = size.min(ORBIT_TAIL);
    let tail = read_at(reader, size - tail_length, tail_length as usize)?;
    Ok(
        if String::from_utf8_lossy(&tail)
            .trim_end()
            .ends_with("</Earth_Explorer_File>")
        {
            ProbeOutcome::Readable
        } else {
            ProbeOutcome::Unreadable
        },
    )
}

fn read_text(reader: &mut File, size: u64) -> io::Result<Option<String>> {
    if size > TEXT_BUDGET {
        return Ok(None);
    }
    let bytes = read_at(reader, 0, size as usize)?;
    Ok(String::from_utf8(bytes).ok())
}

fn probe_metadata_text(reader: &mut File, size: u64) -> io::Result<ProbeOutcome> {
    if size > TEXT_BUDGET {
        return Ok(ProbeOutcome::OverBudget);
    }
    let Some(text) = read_text(reader, size)? else {
        return Ok(ProbeOutcome::Unreadable);
    };
    let mut lines = text.lines().map(str::trim).filter(|line| !line.is_empty());
    let first = lines.next();
    let last = lines.next_back();
    Ok(
        if first.is_some_and(|line| line.starts_with("GROUP = ")) && last == Some("END") {
            ProbeOutcome::Readable
        } else {
            ProbeOutcome::Unreadable
        },
    )
}

fn probe_xml(reader: &mut File, size: u64) -> io::Result<ProbeOutcome> {
    if size > TEXT_BUDGET {
        return Ok(ProbeOutcome::OverBudget);
    }
    let bytes = read_at(reader, 0, size as usize)?;
    let mut parser = quick_xml::Reader::from_reader(bytes.as_slice());
    let mut depth = 0_usize;
    let mut roots = 0_usize;
    let mut buffer = Vec::new();
    loop {
        match parser.read_event_into(&mut buffer) {
            Ok(quick_xml::events::Event::Start(_)) => {
                if depth == 0 {
                    roots += 1;
                }
                depth += 1;
            }
            Ok(quick_xml::events::Event::Empty(_)) if depth == 0 => roots += 1,
            Ok(quick_xml::events::Event::End(_)) => {
                if depth == 0 {
                    return Ok(ProbeOutcome::Unreadable);
                }
                depth -= 1;
            }
            Ok(quick_xml::events::Event::Eof) => break,
            Ok(_) => {}
            Err(_) => return Ok(ProbeOutcome::Unreadable),
        }
        buffer.clear();
    }
    Ok(if roots == 1 && depth == 0 {
        ProbeOutcome::Readable
    } else {
        ProbeOutcome::Unreadable
    })
}

fn probe_json(reader: &mut File, size: u64) -> io::Result<ProbeOutcome> {
    if size > TEXT_BUDGET {
        return Ok(ProbeOutcome::OverBudget);
    }
    let bytes = read_at(reader, 0, size as usize)?;
    Ok(
        if serde_json::from_slice::<serde_json::Value>(&bytes).is_ok() {
            ProbeOutcome::Readable
        } else {
            ProbeOutcome::Unreadable
        },
    )
}
