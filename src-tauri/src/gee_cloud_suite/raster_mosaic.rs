use serde::{Deserialize, Serialize};
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Component, Path};
use std::sync::atomic::{AtomicU64, Ordering};

static GEOTIFF_PART_COUNTER: AtomicU64 = AtomicU64::new(1);

pub const CHUNK_TILE_MAX_PX: u32 = 512;
pub const AUTO_CHUNK_CONFIRM_THRESHOLD: u32 = 9;
pub const HARD_MAX_CHUNKS: u32 = 64;

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ChunkGridPlan {
    pub bbox: [f64; 4],
    pub requested_scale_m: f64,
    pub effective_scale_m: f64,
    pub full_width: u32,
    pub full_height: u32,
    pub chunk_size_px: u32,
    pub chunks_x: u32,
    pub chunks_y: u32,
    pub total_chunks: u32,
    pub scale_x_deg: f64,
    pub scale_y_deg: f64,
    pub estimated_mb: f64,
    pub quota_confirm_required: bool,
    pub suggested_preview_scale_m: f64,
}

#[derive(Clone, Debug)]
pub struct DecodedNpyTile {
    pub width: usize,
    pub height: usize,
    pub band_names: Vec<String>,
    /// Pixel-interleaved `[height * width * bands]` in row-major order.
    pub pixels: Vec<f32>,
}

#[derive(Clone, Debug, Serialize)]
pub struct RasterStatsSummary {
    pub band_count: usize,
    pub band_names: Vec<String>,
    pub valid_pixels: usize,
    pub min: f64,
    pub max: f64,
    pub mean: f64,
}

pub fn plan_chunk_grid(
    bbox: [f64; 4],
    requested_scale_m: f64,
    num_bands_est: usize,
    confirm_full_resolution: bool,
    allow_preview_scale: bool,
) -> Result<ChunkGridPlan, String> {
    let min_lon = bbox[0].min(bbox[2]);
    let max_lon = bbox[0].max(bbox[2]);
    let min_lat = bbox[1].min(bbox[3]);
    let max_lat = bbox[1].max(bbox[3]);
    if !min_lon.is_finite()
        || !max_lon.is_finite()
        || !min_lat.is_finite()
        || !max_lat.is_finite()
        || (max_lon - min_lon) < 1e-6
        || (max_lat - min_lat) < 1e-6
    {
        return Err("INVALID_AOI_BBOX: 研究区经纬度范围无效或面积过小".to_owned());
    }

    let req_scale = requested_scale_m.clamp(1.0, 50_000.0);
    let mid_lat_rad = ((min_lat + max_lat) * 0.5).to_radians();
    let meters_per_deg_lat = 111_320.0_f64;
    let meters_per_deg_lon = (111_320.0_f64 * mid_lat_rad.cos().abs().max(0.08)).max(1000.0);

    let ground_width_m = (max_lon - min_lon) * meters_per_deg_lon;
    let ground_height_m = (max_lat - min_lat) * meters_per_deg_lat;

    let calc_dims = |scale_m: f64| -> (u32, u32, u32, u32, u32) {
        let w = ((ground_width_m / scale_m).ceil() as u64).clamp(4, 32_768) as u32;
        let h = ((ground_height_m / scale_m).ceil() as u64).clamp(4, 32_768) as u32;
        let cx = w.div_ceil(CHUNK_TILE_MAX_PX).max(1);
        let cy = h.div_ceil(CHUNK_TILE_MAX_PX).max(1);
        (w, h, cx, cy, cx.saturating_mul(cy))
    };

    let (raw_w, raw_h, raw_cx, raw_cy, raw_total) = calc_dims(req_scale);

    // Compute a lightweight single-or-4-chunk preview resolution suggestion (<= 1024 px on longest edge)
    let longest_m = ground_width_m.max(ground_height_m);
    let suggested_preview_scale_m = ((longest_m / 1024.0).ceil()).max(req_scale);

    let effective_scale_m = if allow_preview_scale && raw_total > AUTO_CHUNK_CONFIRM_THRESHOLD {
        suggested_preview_scale_m
    } else if confirm_full_resolution && raw_total > HARD_MAX_CHUNKS {
        return Err(format!(
            "GEE_AOI_TOO_LARGE_FOR_SINGLE_DOWNLOAD: 当前研究区按原生 {req_scale} 米分辨率共需请求 {raw_total} 个分块（超出单次在线无损拼接硬上限 {HARD_MAX_CHUNKS} 块），系统拒绝静默降采样篡改科研分辨率。请缩小研究区范围（AOI），或改选推荐的 {suggested_preview_scale_m} 米轻量预览分辨率（use_preview_scale: true）。"
        ));
    } else {
        req_scale
    };

    let (full_w, full_h, cx, cy, total_chunks) = calc_dims(effective_scale_m);
    let quota_confirm_required = !confirm_full_resolution
        && !allow_preview_scale
        && raw_total > AUTO_CHUNK_CONFIRM_THRESHOLD;

    let bands = num_bands_est.clamp(1, 16) as f64;
    let estimated_mb = ((full_w as f64) * (full_h as f64) * bands * 4.0) / (1024.0 * 1024.0);

    let scale_x_deg = (max_lon - min_lon) / (full_w as f64);
    let scale_y_deg = -((max_lat - min_lat) / (full_h as f64));

    Ok(ChunkGridPlan {
        bbox: [min_lon, min_lat, max_lon, max_lat],
        requested_scale_m: req_scale,
        effective_scale_m,
        full_width: if quota_confirm_required {
            raw_w
        } else {
            full_w
        },
        full_height: if quota_confirm_required {
            raw_h
        } else {
            full_h
        },
        chunk_size_px: CHUNK_TILE_MAX_PX,
        chunks_x: if quota_confirm_required { raw_cx } else { cx },
        chunks_y: if quota_confirm_required { raw_cy } else { cy },
        total_chunks: if quota_confirm_required {
            raw_total
        } else {
            total_chunks
        },
        scale_x_deg,
        scale_y_deg,
        estimated_mb: (estimated_mb * 100.0).round() / 100.0,
        quota_confirm_required,
        suggested_preview_scale_m,
    })
}

#[derive(Clone, Debug)]
struct NpyFieldSpec {
    name: String,
    dtype: String,
    byte_len: usize,
}

fn dtype_byte_len(dt: &str) -> Result<usize, String> {
    let clean = dt.trim();
    if clean.len() < 2 {
        return Err(format!("UNSUPPORTED_NPY_DTYPE: {clean}"));
    }
    let size_str = &clean[clean.len() - 1..];
    match size_str {
        "1" => Ok(1),
        "2" => Ok(2),
        "4" => Ok(4),
        "8" => Ok(8),
        _ => Err(format!("UNSUPPORTED_NPY_DTYPE_WIDTH: {clean}")),
    }
}

fn decode_scalar_to_f32(raw: &[u8], dtype: &str) -> f32 {
    let endian_big = dtype.starts_with('>');
    let kind = dtype.as_bytes().get(1).copied().unwrap_or(b'f');
    match (kind, raw.len(), endian_big) {
        (b'u', 1, _) => raw[0] as f32,
        (b'i', 1, _) => (raw[0] as i8) as f32,
        (b'b', 1, _) => {
            if raw[0] != 0 {
                1.0
            } else {
                0.0
            }
        }
        (b'u', 2, false) => u16::from_le_bytes([raw[0], raw[1]]) as f32,
        (b'u', 2, true) => u16::from_be_bytes([raw[0], raw[1]]) as f32,
        (b'i', 2, false) => i16::from_le_bytes([raw[0], raw[1]]) as f32,
        (b'i', 2, true) => i16::from_be_bytes([raw[0], raw[1]]) as f32,
        (b'u', 4, false) => u32::from_le_bytes([raw[0], raw[1], raw[2], raw[3]]) as f32,
        (b'u', 4, true) => u32::from_be_bytes([raw[0], raw[1], raw[2], raw[3]]) as f32,
        (b'i', 4, false) => i32::from_le_bytes([raw[0], raw[1], raw[2], raw[3]]) as f32,
        (b'i', 4, true) => i32::from_be_bytes([raw[0], raw[1], raw[2], raw[3]]) as f32,
        (b'f', 4, false) => f32::from_le_bytes([raw[0], raw[1], raw[2], raw[3]]),
        (b'f', 4, true) => f32::from_be_bytes([raw[0], raw[1], raw[2], raw[3]]),
        (b'f', 8, false) => {
            let mut b = [0u8; 8];
            b.copy_from_slice(&raw[..8]);
            f64::from_le_bytes(b) as f32
        }
        (b'f', 8, true) => {
            let mut b = [0u8; 8];
            b.copy_from_slice(&raw[..8]);
            f64::from_be_bytes(b) as f32
        }
        (b'i', 8, false) => {
            let mut b = [0u8; 8];
            b.copy_from_slice(&raw[..8]);
            i64::from_le_bytes(b) as f32
        }
        _ => f32::NAN,
    }
}

fn extract_quoted_strings(segment: &str) -> Vec<String> {
    let mut res = Vec::new();
    let bytes = segment.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        let q = bytes[i];
        if q == b'\'' || q == b'"' {
            let start = i + 1;
            let mut end = start;
            while end < bytes.len() && bytes[end] != q {
                end += 1;
            }
            if end < bytes.len() {
                res.push(segment[start..end].to_owned());
                i = end + 1;
                continue;
            }
        }
        i += 1;
    }
    res
}

fn parse_npy_header(header: &str) -> Result<(usize, usize, Vec<NpyFieldSpec>), String> {
    if header.contains("'fortran_order': True") || header.contains("\"fortran_order\": True") {
        return Err("UNSUPPORTED_NPY_FORTRAN_ORDER: 不支持 Fortran 列优先数组".to_owned());
    }

    // Parse shape: (H, W) or (H, W, C)
    let shape_idx = header
        .find("'shape'")
        .or_else(|| header.find("\"shape\""))
        .ok_or_else(|| format!("INVALID_NPY_HEADER_NO_SHAPE: {header}"))?;
    let after_shape = &header[shape_idx..];
    let lparen = after_shape
        .find('(')
        .ok_or_else(|| format!("INVALID_NPY_SHAPE: {header}"))?;
    let rparen = after_shape[lparen..]
        .find(')')
        .ok_or_else(|| format!("INVALID_NPY_SHAPE_CLOSE: {header}"))?
        + lparen;
    let dims: Vec<usize> = after_shape[lparen + 1..rparen]
        .split(',')
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(|s| {
            s.parse::<usize>()
                .map_err(|e| format!("INVALID_NPY_DIM ({s}): {e}"))
        })
        .collect::<Result<Vec<_>, _>>()?;
    if dims.len() < 2 {
        return Err(format!("INVALID_NPY_DIMS_COUNT: {dims:?}"));
    }
    let height = dims[0];
    let width = dims[1];

    // Parse descr: either [('B4', '<u2'), ('B3', '<u2')] OR '<f4'
    let descr_idx = header
        .find("'descr'")
        .or_else(|| header.find("\"descr\""))
        .ok_or_else(|| format!("INVALID_NPY_HEADER_NO_DESCR: {header}"))?;
    let after_descr = &header[descr_idx + 7..];
    let colon = after_descr
        .find(':')
        .ok_or_else(|| format!("INVALID_NPY_DESCR_COLON: {header}"))?;
    let val_part = after_descr[colon + 1..].trim_start();

    let mut fields = Vec::new();
    if val_part.starts_with('[') {
        let rbracket = val_part
            .find(']')
            .ok_or_else(|| format!("INVALID_NPY_STRUCTURED_DESCR: {header}"))?;
        let list_body = &val_part[1..rbracket];
        let mut cursor = 0;
        while let Some(rel_lp) = list_body[cursor..].find('(') {
            let start_p = cursor + rel_lp;
            let end_p = list_body[start_p..]
                .find(')')
                .ok_or_else(|| format!("INVALID_NPY_TUPLE: {list_body}"))?
                + start_p;
            let tuple_str = &list_body[start_p + 1..end_p];
            let quoted = extract_quoted_strings(tuple_str);
            if quoted.len() >= 2 {
                let b_len = dtype_byte_len(&quoted[1])?;
                fields.push(NpyFieldSpec {
                    name: quoted[0].clone(),
                    dtype: quoted[1].clone(),
                    byte_len: b_len,
                });
            }
            cursor = end_p + 1;
        }
    } else {
        let quoted = extract_quoted_strings(val_part);
        let dt = quoted
            .first()
            .ok_or_else(|| format!("INVALID_NPY_SCALAR_DESCR: {header}"))?
            .clone();
        let b_len = dtype_byte_len(&dt)?;
        let channels = if dims.len() >= 3 { dims[2].max(1) } else { 1 };
        for c in 0..channels {
            fields.push(NpyFieldSpec {
                name: format!("band_{}", c + 1),
                dtype: dt.clone(),
                byte_len: b_len,
            });
        }
    }

    if fields.is_empty() {
        return Err(format!("EMPTY_NPY_FIELDS: {header}"));
    }
    Ok((height, width, fields))
}

pub fn decode_npy_tile(bytes: &[u8]) -> Result<DecodedNpyTile, String> {
    if bytes.len() < 10 || &bytes[0..6] != b"\x93NUMPY" {
        return Err("INVALID_NPY_MAGIC: GEE 响应不是合法的 NPY 数组格式".to_owned());
    }
    let major = bytes[6];
    let (hdr_offset, hdr_len) = match major {
        1 => {
            let len = u16::from_le_bytes([bytes[8], bytes[9]]) as usize;
            (10usize, len)
        }
        2 | 3 => {
            if bytes.len() < 12 {
                return Err("INVALID_NPY_V2_HEADER: NPY 头部截断".to_owned());
            }
            let len = u32::from_le_bytes([bytes[8], bytes[9], bytes[10], bytes[11]]) as usize;
            (12usize, len)
        }
        _ => return Err(format!("UNSUPPORTED_NPY_VERSION: v{major}")),
    };
    if bytes.len() < hdr_offset + hdr_len {
        return Err("TRUNCATED_NPY_HEADER: NPY 头部长度超出实际字节".to_owned());
    }
    let header_str = std::str::from_utf8(&bytes[hdr_offset..hdr_offset + hdr_len])
        .map_err(|e| format!("INVALID_NPY_UTF8: {e}"))?;
    let (height, width, fields) = parse_npy_header(header_str)?;

    let record_bytes: usize = fields.iter().map(|f| f.byte_len).sum();
    let expected_data_bytes = height
        .checked_mul(width)
        .and_then(|px| px.checked_mul(record_bytes))
        .ok_or_else(|| "NPY_OVERFLOW: 栅格块尺寸溢出".to_owned())?;
    let data = &bytes[hdr_offset + hdr_len..];
    if data.len() < expected_data_bytes {
        return Err(format!(
            "TRUNCATED_NPY_DATA: 期望 {expected_data_bytes} 字节，实际 {} 字节",
            data.len()
        ));
    }

    let num_bands = fields.len();
    let mut pixels = Vec::with_capacity(height * width * num_bands);
    let mut offset = 0usize;
    for _ in 0..(height * width) {
        for field in &fields {
            let val = decode_scalar_to_f32(&data[offset..offset + field.byte_len], &field.dtype);
            pixels.push(val);
            offset += field.byte_len;
        }
    }

    Ok(DecodedNpyTile {
        width,
        height,
        band_names: fields.into_iter().map(|f| f.name).collect(),
        pixels,
    })
}

/// Write a standards-compliant uncompressed little-endian IEEE-754 float32 GeoTIFF (`II*\0`)
/// with EPSG:4326 geographic tiepoint & pixel scale tags (`33550`, `33922`, `34735`).
pub fn write_geotiff_f32_epsg4326(
    out_path: &Path,
    width: u32,
    height: u32,
    num_bands: u16,
    bbox: [f64; 4],
    pixels: &[f32],
) -> Result<u64, String> {
    let expected_len = (width as usize) * (height as usize) * (num_bands as usize);
    if pixels.len() != expected_len {
        return Err(format!(
            "GEOTIFF_BUFFER_MISMATCH: 期望 {expected_len} 个像素值，实际 {}",
            pixels.len()
        ));
    }
    let min_lon = bbox[0].min(bbox[2]);
    let max_lon = bbox[0].max(bbox[2]);
    let min_lat = bbox[1].min(bbox[3]);
    let max_lat = bbox[1].max(bbox[3]);
    let scale_x = (max_lon - min_lon) / (width as f64);
    let scale_y = (max_lat - min_lat) / (height as f64);

    let pixel_bytes_len = (expected_len * 4) as u32;

    // Layout:
    // [0..8]: Header ("II*\0" + IFD offset = 8)
    // [8..8 + 2 + 14*12 + 4]: 14 ascending TIFF/GeoTIFF tags
    // Extra value blocks immediately after IFD, then pixel strip data
    let entry_count: u16 = 14;
    let ifd_offset: u32 = 8;
    let ifd_bytes_len: u32 = 2 + (entry_count as u32) * 12 + 4;
    let mut aux_offset: u32 = ifd_offset + ifd_bytes_len;

    // Aux blocks:
    // 1. BitsPerSample if num_bands > 2 (u16 * num_bands)
    // 2. SampleFormat if num_bands > 2 (u16 * num_bands)
    // 3. ModelPixelScaleTag (3 * f64 = 24 bytes)
    // 4. ModelTiepointTag (6 * f64 = 48 bytes)
    // 5. GeoKeyDirectoryTag (16 * u16 = 32 bytes)
    let mut aux_buf: Vec<u8> = Vec::with_capacity(256);

    let (bps_count, bps_val_or_off) = if num_bands == 1 {
        (1u32, 32u32)
    } else if num_bands == 2 {
        (2u32, 32u32 | (32u32 << 16))
    } else {
        let off = aux_offset;
        for _ in 0..num_bands {
            aux_buf.extend_from_slice(&32u16.to_le_bytes());
        }
        if !aux_buf.len().is_multiple_of(2) {
            aux_buf.push(0);
        }
        aux_offset = ifd_offset + ifd_bytes_len + (aux_buf.len() as u32);
        (num_bands as u32, off)
    };

    let (sf_count, sf_val_or_off) = if num_bands == 1 {
        (1u32, 3u32) // 3 = IEEE floating point
    } else if num_bands == 2 {
        (2u32, 3u32 | (3u32 << 16))
    } else {
        let off = aux_offset;
        for _ in 0..num_bands {
            aux_buf.extend_from_slice(&3u16.to_le_bytes());
        }
        (num_bands as u32, off)
    };

    // Align aux_buf to 8 bytes before f64 arrays
    while !(ifd_offset + ifd_bytes_len + aux_buf.len() as u32).is_multiple_of(8) {
        aux_buf.push(0);
    }
    aux_offset = ifd_offset + ifd_bytes_len + (aux_buf.len() as u32);

    let pixel_scale_off = aux_offset;
    for v in [scale_x, scale_y, 0.0_f64] {
        aux_buf.extend_from_slice(&v.to_le_bytes());
    }
    aux_offset = ifd_offset + ifd_bytes_len + (aux_buf.len() as u32);

    let tiepoint_off = aux_offset;
    for v in [0.0_f64, 0.0_f64, 0.0_f64, min_lon, max_lat, 0.0_f64] {
        aux_buf.extend_from_slice(&v.to_le_bytes());
    }
    aux_offset = ifd_offset + ifd_bytes_len + (aux_buf.len() as u32);

    // GeoKeyDirectoryTag: EPSG:4326 (GTModelTypeGeoKey=2 Geographic, GTRasterTypeGeoKey=1 PixelIsArea, GeographicTypeGeoKey=4326)
    let geokey_off = aux_offset;
    let geokeys: [u16; 16] = [
        1, 1, 0, 3, // Header: KeyDirectoryVersion=1, KeyRevision=1.0, NumberOfKeys=3
        1024, 0, 1, 2, // GTModelTypeGeoKey = ModelTypeGeographic (2)
        1025, 0, 1, 1, // GTRasterTypeGeoKey = RasterPixelIsArea (1)
        2048, 0, 1, 4326, // GeographicTypeGeoKey = GCS_WGS_84 (4326)
    ];
    for gk in geokeys {
        aux_buf.extend_from_slice(&gk.to_le_bytes());
    }
    while !(ifd_offset + ifd_bytes_len + aux_buf.len() as u32).is_multiple_of(4) {
        aux_buf.push(0);
    }
    let strip_offset: u32 = ifd_offset + ifd_bytes_len + (aux_buf.len() as u32);

    // PhotometricInterpretation: 1 (BlackIsZero) for 1/2/4+ bands, 2 (RGB) when 3 bands? Keep 1 (MinIsBlack) so scientific float32 values are never clamped by GIS readers!
    let photometric: u32 = 1;

    // Ascending tag order is REQUIRED by TIFF 6.0 spec!
    // Types: 3 = SHORT (u16), 4 = LONG (u32), 12 = DOUBLE (f64)
    let tags: [(u16, u16, u32, u32); 14] = [
        (256, 4, 1, width),                  // ImageWidth
        (257, 4, 1, height),                 // ImageLength
        (258, 3, bps_count, bps_val_or_off), // BitsPerSample
        (259, 3, 1, 1),                      // Compression = None (1)
        (262, 3, 1, photometric),            // PhotometricInterpretation = BlackIsZero
        (273, 4, 1, strip_offset),           // StripOffsets
        (277, 3, 1, num_bands as u32),       // SamplesPerPixel
        (278, 4, 1, height),                 // RowsPerStrip
        (279, 4, 1, pixel_bytes_len),        // StripByteCounts
        (284, 3, 1, 1),                      // PlanarConfiguration = Contiguous (1)
        (339, 3, sf_count, sf_val_or_off),   // SampleFormat = IEEEFP (3)
        (33550, 12, 3, pixel_scale_off),     // ModelPixelScaleTag
        (33922, 12, 6, tiepoint_off),        // ModelTiepointTag
        (34735, 3, 16, geokey_off),          // GeoKeyDirectoryTag
    ];

    let total_file_bytes = (strip_offset as usize) + (pixel_bytes_len as usize);
    let mut out = Vec::with_capacity(total_file_bytes);
    out.extend_from_slice(b"II*\0");
    out.extend_from_slice(&ifd_offset.to_le_bytes());
    out.extend_from_slice(&entry_count.to_le_bytes());
    for (tag, field_type, count, val_off) in tags {
        out.extend_from_slice(&tag.to_le_bytes());
        out.extend_from_slice(&field_type.to_le_bytes());
        out.extend_from_slice(&count.to_le_bytes());
        out.extend_from_slice(&val_off.to_le_bytes());
    }
    out.extend_from_slice(&0u32.to_le_bytes()); // next IFD = 0
    out.extend_from_slice(&aux_buf);
    debug_assert_eq!(out.len(), strip_offset as usize);

    for &px in pixels {
        out.extend_from_slice(&px.to_le_bytes());
    }

    if out_path.as_os_str().is_empty()
        || out_path.file_name().is_none()
        || out_path
            .components()
            .any(|c| matches!(c, Component::ParentDir))
    {
        return Err(
            "UNSAFE_GEOTIFF_OUTPUT_PATH: 拒绝包含路径穿越(..)或缺少文件名的目标路径".to_owned(),
        );
    }
    if let Some(parent) = out_path.parent() {
        if !parent.as_os_str().is_empty() {
            if fs::symlink_metadata(parent).is_ok_and(|m| m.file_type().is_symlink()) {
                return Err(
                    "UNSAFE_GEOTIFF_OUTPUT_PATH: 拒绝通过符号链接目录写入目标路径".to_owned(),
                );
            }
            fs::create_dir_all(parent).map_err(|e| format!("CREATE_GEOTIFF_DIR_FAILED: {e}"))?;
            if fs::symlink_metadata(parent).is_ok_and(|m| m.file_type().is_symlink()) {
                return Err(
                    "UNSAFE_GEOTIFF_OUTPUT_PATH: 拒绝通过符号链接目录写入目标路径".to_owned(),
                );
            }
        }
    }
    let part_seq = GEOTIFF_PART_COUNTER.fetch_add(1, Ordering::Relaxed);
    let part_path = out_path.with_extension(format!("tif.part-{}-{part_seq}", std::process::id()));
    {
        let mut part_file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&part_path)
            .map_err(|e| format!("WRITE_GEOTIFF_PART_FAILED: {e}"))?;
        part_file.write_all(&out).map_err(|e| {
            let _ = fs::remove_file(&part_path);
            format!("WRITE_GEOTIFF_PART_FAILED: {e}")
        })?;
        part_file.sync_all().map_err(|e| {
            let _ = fs::remove_file(&part_path);
            format!("SYNC_GEOTIFF_PART_FAILED: {e}")
        })?;
    }

    match fs::hard_link(&part_path, out_path) {
        Ok(()) => {
            let _ = fs::remove_file(&part_path);
        }
        Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {
            let _ = fs::remove_file(&part_path);
            return Err(format!(
                "GEOTIFF_TARGET_ALREADY_EXISTS: 目标文件已存在，拒绝覆盖 ({})",
                out_path.display()
            ));
        }
        Err(_) => {
            let _ = fs::remove_file(&part_path);
            let mut excl_file = OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(out_path)
                .map_err(|e| {
                    if e.kind() == std::io::ErrorKind::AlreadyExists {
                        format!(
                            "GEOTIFF_TARGET_ALREADY_EXISTS: 目标文件已存在，拒绝覆盖 ({})",
                            out_path.display()
                        )
                    } else {
                        format!("PROMOTE_GEOTIFF_FAILED: {e}")
                    }
                })?;
            excl_file.write_all(&out).map_err(|e| {
                let _ = fs::remove_file(out_path);
                format!("PROMOTE_GEOTIFF_FAILED: {e}")
            })?;
            excl_file.sync_all().map_err(|e| {
                let _ = fs::remove_file(out_path);
                format!("PROMOTE_GEOTIFF_FAILED: {e}")
            })?;
        }
    }

    Ok(out.len() as u64)
}

pub fn summarize_raster_stats(pixels: &[f32], band_names: &[String]) -> RasterStatsSummary {
    let mut min_v = f64::INFINITY;
    let mut max_v = f64::NEG_INFINITY;
    let mut sum_v = 0.0_f64;
    let mut valid_count = 0usize;

    for &v in pixels {
        let vf = v as f64;
        if vf.is_finite() {
            if vf < min_v {
                min_v = vf;
            }
            if vf > max_v {
                max_v = vf;
            }
            sum_v += vf;
            valid_count += 1;
        }
    }

    if valid_count == 0 {
        min_v = 0.0;
        max_v = 0.0;
    }
    let mean_v = if valid_count > 0 {
        sum_v / (valid_count as f64)
    } else {
        0.0
    };

    RasterStatsSummary {
        band_count: band_names.len().max(1),
        band_names: band_names.to_vec(),
        valid_pixels: valid_count,
        min: (min_v * 10000.0).round() / 10000.0,
        max: (max_v * 10000.0).round() / 10000.0,
        mean: (mean_v * 10000.0).round() / 10000.0,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plan_chunk_grid_preserves_full_resolution_up_to_64_and_rejects_above_64_without_preview() {
        // Medium AOI (~20 km x ~22 km at 10m -> 4x5 = 20 chunks, within 9..=64)
        let medium_bbox = [117.10, 31.70, 117.31, 31.90];
        let pre = plan_chunk_grid(medium_bbox, 10.0, 1, false, false).unwrap();
        assert!(pre.quota_confirm_required);
        assert!(
            pre.total_chunks > AUTO_CHUNK_CONFIRM_THRESHOLD && pre.total_chunks <= HARD_MAX_CHUNKS
        );
        assert_eq!(pre.effective_scale_m, 10.0);

        let confirmed = plan_chunk_grid(medium_bbox, 10.0, 1, true, false).unwrap();
        assert!(!confirmed.quota_confirm_required);
        assert_eq!(confirmed.effective_scale_m, 10.0);
        assert_eq!(confirmed.total_chunks, pre.total_chunks);

        // Huge AOI (~75 km x ~78 km at 10m -> > 64 chunks): NEVER silently coarsen!
        let huge_bbox = [117.0, 31.5, 117.8, 32.2];
        let err = plan_chunk_grid(huge_bbox, 10.0, 3, true, false)
            .expect_err("confirm_full_resolution above 64 chunks must fail explicitly");
        assert!(
            err.contains("GEE_AOI_TOO_LARGE_FOR_SINGLE_DOWNLOAD"),
            "{err}"
        );
        assert!(err.contains("use_preview_scale: true"), "{err}");

        // Explicit preview scale succeeds cleanly within chunk ceiling
        let preview = plan_chunk_grid(huge_bbox, 10.0, 3, false, true).unwrap();
        assert!(!preview.quota_confirm_required);
        assert_eq!(preview.effective_scale_m, preview.suggested_preview_scale_m);
        assert!(preview.effective_scale_m > 10.0);
        assert!(preview.total_chunks <= HARD_MAX_CHUNKS);
    }
}
