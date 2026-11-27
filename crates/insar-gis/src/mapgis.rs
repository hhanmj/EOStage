use super::{NativeGisError, NativeGisErrorCode};
use serde::{Deserialize, Serialize};
use serde_json::{json, Map, Value};
use std::collections::BTreeMap;
use std::fs;
use std::path::Path;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum MapGisShapeKind {
    Point,
    Line,
    Polygon,
}

impl MapGisShapeKind {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Point => "POINT",
            Self::Line => "LINE",
            Self::Polygon => "POLYGON",
        }
    }

    pub const fn geojson_type(self) -> &'static str {
        match self {
            Self::Point => "Point",
            Self::Line => "LineString",
            Self::Polygon => "Polygon",
        }
    }
}

#[derive(Clone, Debug)]
pub struct ParsedMapGisDataset {
    pub kind: MapGisShapeKind,
    pub format_variant: String,
    pub crs_proj4: Option<String>,
    pub crs_wkt: Option<String>,
    pub coordinate_scale: f64,
    pub fields: Vec<(String, String, usize)>,
    pub features: Vec<ParsedMapGisFeature>,
    pub bbox: Option<[f64; 4]>,
}

#[derive(Clone, Debug)]
pub struct ParsedMapGisFeature {
    pub id: usize,
    pub coords: Vec<Vec<[f64; 2]>>,
    pub properties: Map<String, Value>,
}

fn error(code: NativeGisErrorCode, reason: impl Into<String>) -> NativeGisError {
    NativeGisError::contract(code, reason)
}

fn read_i16_le(bytes: &[u8], offset: usize) -> Result<i16, NativeGisError> {
    let slice = bytes.get(offset..offset + 2).ok_or_else(|| {
        error(
            NativeGisErrorCode::AoiFormatUnsupported,
            "MapGIS 二进制文件意外截断 (i16)",
        )
    })?;
    Ok(i16::from_le_bytes([slice[0], slice[1]]))
}

#[allow(dead_code)]
fn read_u16_le(bytes: &[u8], offset: usize) -> Result<u16, NativeGisError> {
    let slice = bytes.get(offset..offset + 2).ok_or_else(|| {
        error(
            NativeGisErrorCode::AoiFormatUnsupported,
            "MapGIS 二进制文件意外截断 (u16)",
        )
    })?;
    Ok(u16::from_le_bytes([slice[0], slice[1]]))
}

fn read_i32_le(bytes: &[u8], offset: usize) -> Result<i32, NativeGisError> {
    let slice = bytes.get(offset..offset + 4).ok_or_else(|| {
        error(
            NativeGisErrorCode::AoiFormatUnsupported,
            "MapGIS 二进制文件意外截断 (i32)",
        )
    })?;
    Ok(i32::from_le_bytes([slice[0], slice[1], slice[2], slice[3]]))
}

fn read_f32_le(bytes: &[u8], offset: usize) -> Result<f32, NativeGisError> {
    let slice = bytes.get(offset..offset + 4).ok_or_else(|| {
        error(
            NativeGisErrorCode::AoiFormatUnsupported,
            "MapGIS 二进制文件意外截断 (f32)",
        )
    })?;
    Ok(f32::from_le_bytes([slice[0], slice[1], slice[2], slice[3]]))
}

fn read_f64_le(bytes: &[u8], offset: usize) -> Result<f64, NativeGisError> {
    let slice = bytes.get(offset..offset + 8).ok_or_else(|| {
        error(
            NativeGisErrorCode::AoiFormatUnsupported,
            "MapGIS 二进制文件意外截断 (f64)",
        )
    })?;
    Ok(f64::from_le_bytes([
        slice[0], slice[1], slice[2], slice[3], slice[4], slice[5], slice[6], slice[7],
    ]))
}

fn decode_c_string(raw: &[u8]) -> String {
    let end = raw.iter().position(|&b| b == 0).unwrap_or(raw.len());
    let trimmed = &raw[..end];
    if let Ok(utf8) = std::str::from_utf8(trimmed) {
        return utf8.trim().to_owned();
    }
    let mut out = String::new();
    for &b in trimmed {
        if b.is_ascii_graphic() || b == b' ' {
            out.push(b as char);
        } else {
            out.push_str(&format!("x{b:02X}"));
        }
    }
    out.trim().to_owned()
}

fn parse_mapgis_crs(
    bytes: &[u8],
    scale_override: Option<f64>,
) -> (f64, Option<String>, Option<String>) {
    if bytes.len() < 160 {
        return (scale_override.unwrap_or(1.0), None, None);
    }
    let proj_type = bytes[109];
    let ellipsoid = bytes[110];
    let raw_scale = read_f64_le(bytes, 143).unwrap_or(1.0);
    let mut scale = scale_override.unwrap_or(if raw_scale.is_finite() && raw_scale.abs() > 1e-12 {
        raw_scale
    } else {
        1.0
    });
    if matches!(proj_type, 2 | 3) && scale_override.is_none() {
        scale /= 1000.0;
    }
    let ellip_proj4 = match ellipsoid {
        1 => Some("+ellps=krass +towgs84=15.8,-154.4,-82.3,0,0,0,0"),
        2 => Some("+a=6378140 +b=6356755.288157528"),
        7 => Some("+datum=WGS84"),
        9 => Some("+ellps=WGS72"),
        10 => Some("+ellps=aust_SA +towgs84=-117.808,-51.536,137.784,0.303,0.446,0.234,-0.29"),
        11 => Some("+ellps=aust_SA +towgs84=-134,-48,149,0,0,0,0"),
        16 => Some("+ellps=krass"),
        116 => Some("+ellps=clrk80 +towgs84=-166,-15,204,0,0,0,0"),
        _ => None,
    };
    let Some(ellip) = ellip_proj4 else {
        return (scale, None, None);
    };
    if proj_type == 0 {
        let proj4 = format!("+proj=longlat {ellip} +no_defs");
        let wkt = r#"GEOGCS["WGS 84",DATUM["WGS_1984",SPHEROID["WGS 84",6378137,298.257223563]],PRIMEM["Greenwich",0],UNIT["degree",0.0174532925199433],AUTHORITY["EPSG","4326"]]"#.to_owned();
        return (scale, Some(proj4), Some(wkt));
    }
    if proj_type == 5 {
        let raw_cl = read_f64_le(bytes, 151).unwrap_or(1170000.0);
        let cl_int = raw_cl.abs().trunc() as i64;
        let deg = (cl_int / 10000) as f64;
        let min = ((cl_int % 10000) / 100) as f64;
        let sec = (cl_int % 100) as f64;
        let lon_0 = if cl_int >= 10000 {
            deg + min / 60.0 + sec / 3600.0
        } else {
            raw_cl
        };
        let proj4 = format!(
            "+proj=tmerc +lat_0=0 +lon_0={lon_0} +k=1 +x_0=500000 +y_0=0 {ellip} +units=m +no_defs"
        );
        return (scale, Some(proj4), None);
    }
    (scale, None, None)
}

#[allow(clippy::type_complexity)]
fn parse_binary_attributes(
    bytes: &[u8],
    attr_start: usize,
) -> Result<(Vec<(String, String, usize)>, Vec<Map<String, Value>>), NativeGisError> {
    if attr_start == 0 || attr_start + 344 > bytes.len() {
        return Ok((Vec::new(), Vec::new()));
    }
    let header_base = attr_start + 318;
    let field_count = read_i16_le(bytes, header_base)?.max(0) as usize;
    let record_count = read_i32_le(bytes, header_base + 2)?.max(0) as usize;
    let record_length = read_i16_le(bytes, header_base + 6)?.max(0) as usize;
    let mut cursor = header_base + 26;
    if cursor + field_count * 39 > bytes.len() {
        return Ok((Vec::new(), Vec::new()));
    }
    let mut descriptors = Vec::new();
    for idx in 0..field_count {
        let raw_name = &bytes[cursor..cursor + 20];
        let mut name = decode_c_string(raw_name);
        if name.is_empty() {
            name = format!("FIELD_{}", idx + 1);
        }
        let ftype = bytes[cursor + 20];
        let foffset = read_i32_le(bytes, cursor + 21)?.max(0) as usize;
        let flen = read_i16_le(bytes, cursor + 27)?.max(0) as usize;
        cursor += 39;
        if ftype <= 7 && foffset < record_length {
            descriptors.push((name, ftype, foffset, flen));
        }
    }
    // Skip dummy record 0
    cursor += record_length;
    let mut fields_meta = Vec::new();
    for i in 0..descriptors.len() {
        let (ref name, ftype, foffset, flen) = descriptors[i];
        let next_offset = descriptors.get(i + 1).map(|d| d.2).unwrap_or(record_length);
        let actual_len = next_offset.saturating_sub(foffset).max(flen);
        let type_name = match ftype {
            0 => "string",
            1 => "byte",
            2 => "short",
            3 => "int",
            4 => "float",
            5 => "double",
            6 => "date",
            7 => "time",
            _ => "string",
        };
        fields_meta.push((name.clone(), type_name.to_owned(), actual_len));
    }

    let mut rows = Vec::new();
    if record_count <= 1 || record_length == 0 {
        return Ok((fields_meta, rows));
    }
    for _ in 0..(record_count - 1) {
        if cursor + record_length > bytes.len() {
            break;
        }
        let row_bytes = &bytes[cursor..cursor + record_length];
        cursor += record_length;
        if row_bytes[0] != 1 {
            continue;
        }
        let mut props = Map::new();
        for i in 0..descriptors.len() {
            let (ref name, ftype, foffset, _) = descriptors[i];
            let end_offset = descriptors
                .get(i + 1)
                .map(|d| d.2)
                .unwrap_or(record_length)
                .min(record_length);
            if foffset >= end_offset {
                continue;
            }
            let slice = &row_bytes[foffset..end_offset];
            let val = match ftype {
                0 => Value::String(decode_c_string(slice)),
                1 => Value::from(slice[0]),
                2 => Value::from(read_i16_le(slice, 0).unwrap_or(0)),
                3 => Value::from(read_i32_le(slice, 0).unwrap_or(0)),
                4 => Value::from(f64::from(read_f32_le(slice, 0).unwrap_or(0.0))),
                5 => Value::from(read_f64_le(slice, 0).unwrap_or(0.0)),
                _ => Value::String(decode_c_string(slice)),
            };
            props.insert(name.clone(), val);
        }
        rows.push(props);
    }
    Ok((fields_meta, rows))
}

pub fn parse_mapgis_bytes(
    bytes: &[u8],
    scale_override: Option<f64>,
) -> Result<ParsedMapGisDataset, NativeGisError> {
    if bytes.len() < 16 {
        return Err(error(
            NativeGisErrorCode::AoiFormatUnsupported,
            "MapGIS 文件字节数不足，无法识别文件头",
        ));
    }
    // Check if it is MapGIS 6.x Clear-Text (明码) format (`WMAP_9022` / `WPNT` / `WPLN` / `WPOL`)
    if let Ok(text_head) = std::str::from_utf8(&bytes[..bytes.len().min(64)]) {
        let trimmed = text_head.trim_start();
        if trimmed.starts_with("WMAP_9022")
            || trimmed.starts_with("WPNT")
            || trimmed.starts_with("WPLN")
            || trimmed.starts_with("WPOL")
        {
            return parse_mapgis_clear_text(bytes, scale_override);
        }
    }

    let magic = &bytes[..8];
    let kind = match magic {
        b"WMAP`D22" => MapGisShapeKind::Point,
        b"WMAP`D21" => MapGisShapeKind::Line,
        b"WMAP`D23" => MapGisShapeKind::Polygon,
        _ => {
            return Err(error(
                NativeGisErrorCode::AoiFormatUnsupported,
                "文件头不匹配 MapGIS 6.x 二进制格式 (期望 WMAP`D21 / WMAP`D22 / WMAP`D23 或明码 WMAP_9022)",
            ))
        }
    };
    let data_start = read_i32_le(bytes, 12)?.max(0) as usize;
    if data_start + 100 > bytes.len() {
        return Err(error(
            NativeGisErrorCode::AoiFormatUnsupported,
            "MapGIS 数据段目录偏移超出文件长度",
        ));
    }
    let mut headers = [(0usize, 0usize); 10];
    for (idx, slot) in headers.iter_mut().enumerate() {
        let off = data_start + idx * 10;
        let sec_start = read_i32_le(bytes, off)?.max(0) as usize;
        let sec_vol = read_i32_le(bytes, off + 4)?.max(0) as usize;
        *slot = (sec_start, sec_vol);
    }

    let (scale, crs_proj4, crs_wkt) = parse_mapgis_crs(bytes, scale_override);
    let attr_sec = match kind {
        MapGisShapeKind::Point | MapGisShapeKind::Line => headers[2].0,
        MapGisShapeKind::Polygon => headers[9].0,
    };
    let (mut fields, attr_rows) = parse_binary_attributes(bytes, attr_sec)?;

    let mut features = Vec::new();
    match kind {
        MapGisShapeKind::Point => {
            let (pt_start, pt_vol) = headers[0];
            let count = (pt_vol / 93).saturating_sub(1);
            let mut valid_idx = 0usize;
            for i in 0..count {
                let rec_off = pt_start + 93 * (i + 1);
                if rec_off + 93 > bytes.len() {
                    break;
                }
                if bytes[rec_off] != 1 {
                    continue;
                }
                let x = read_f64_le(bytes, rec_off + 7)? * scale;
                let y = read_f64_le(bytes, rec_off + 15)? * scale;
                let point_type = bytes[rec_off + 31];
                let color = read_i32_le(bytes, rec_off + 75).unwrap_or(0);
                let layer = read_i16_le(bytes, rec_off + 83).unwrap_or(0);
                let mut props = attr_rows.get(valid_idx).cloned().unwrap_or_default();
                props
                    .entry("ID".to_owned())
                    .or_insert_with(|| Value::from(valid_idx as i64));
                props.insert("PntType".to_owned(), Value::from(point_type));
                props.insert("Color".to_owned(), Value::from(color));
                props.insert("Layer".to_owned(), Value::from(layer));
                features.push(ParsedMapGisFeature {
                    id: valid_idx,
                    coords: vec![vec![[x, y]]],
                    properties: props,
                });
                valid_idx += 1;
            }
        }
        MapGisShapeKind::Line => {
            let (ln_start, ln_vol) = headers[0];
            let (xy_start, _) = headers[1];
            let count = (ln_vol / 57).saturating_sub(1);
            let mut valid_idx = 0usize;
            for i in 0..count {
                let rec_off = ln_start + 57 * (i + 1);
                if rec_off + 57 > bytes.len() {
                    break;
                }
                if bytes[rec_off] != 1 {
                    continue;
                }
                let node_count = read_i32_le(bytes, rec_off + 10)?.max(0) as usize;
                let node_offset = read_i32_le(bytes, rec_off + 14)?.max(0) as usize;
                let line_type = read_i32_le(bytes, rec_off + 22).unwrap_or(0);
                let line_color = read_i32_le(bytes, rec_off + 26).unwrap_or(0);
                let line_width = read_f32_le(bytes, rec_off + 30).unwrap_or(0.0);
                let layer = read_i16_le(bytes, rec_off + 47).unwrap_or(0);
                if node_count < 2 || xy_start + node_offset + node_count * 16 > bytes.len() {
                    continue;
                }
                let mut pts = Vec::with_capacity(node_count);
                for p in 0..node_count {
                    let poff = xy_start + node_offset + p * 16;
                    let x = read_f64_le(bytes, poff)? * scale;
                    let y = read_f64_le(bytes, poff + 8)? * scale;
                    pts.push([x, y]);
                }
                let mut props = attr_rows.get(valid_idx).cloned().unwrap_or_default();
                props
                    .entry("ID".to_owned())
                    .or_insert_with(|| Value::from(valid_idx as i64));
                props.insert("LineType".to_owned(), Value::from(line_type));
                props.insert("LineCol".to_owned(), Value::from(line_color));
                props.insert("LineWid".to_owned(), Value::from(f64::from(line_width)));
                props.insert("Layer".to_owned(), Value::from(layer));
                features.push(ParsedMapGisFeature {
                    id: valid_idx,
                    coords: vec![pts],
                    properties: props,
                });
                valid_idx += 1;
            }
        }
        MapGisShapeKind::Polygon => {
            let (arc_start, arc_vol) = headers[0];
            let (xy_start, _) = headers[1];
            let (topo_start, topo_vol) = headers[3];
            let arc_count = (arc_vol / 57).saturating_sub(1);
            let mut arcs: Vec<Vec<[f64; 2]>> = Vec::with_capacity(arc_count);
            for i in 0..arc_count {
                let rec_off = arc_start + 57 * (i + 1);
                if rec_off + 57 > bytes.len() {
                    break;
                }
                let node_count = read_i32_le(bytes, rec_off + 10)?.max(0) as usize;
                let node_offset = read_i32_le(bytes, rec_off + 14)?.max(0) as usize;
                let mut pts = Vec::new();
                if node_count >= 2 && xy_start + node_offset + node_count * 16 <= bytes.len() {
                    for p in 0..node_count {
                        let poff = xy_start + node_offset + p * 16;
                        let x = read_f64_le(bytes, poff)? * scale;
                        let y = read_f64_le(bytes, poff + 8)? * scale;
                        pts.push([x, y]);
                    }
                }
                arcs.push(pts);
            }

            let topo_count = (topo_vol / 24).saturating_sub(1);
            let mut poly_arcs: BTreeMap<i32, Vec<Vec<[f64; 2]>>> = BTreeMap::new();
            for i in 0..topo_count {
                let toff = topo_start + 24 * (i + 1);
                if toff + 24 > bytes.len() {
                    break;
                }
                let left_poly = read_i32_le(bytes, toff + 8)?;
                let right_poly = read_i32_le(bytes, toff + 12)?;
                let Some(arc_pts) = arcs.get(i).filter(|pts| pts.len() >= 2) else {
                    continue;
                };
                if left_poly > 0 {
                    let mut rev = arc_pts.clone();
                    rev.reverse();
                    poly_arcs.entry(left_poly).or_default().push(rev);
                }
                if right_poly > 0 {
                    poly_arcs
                        .entry(right_poly)
                        .or_default()
                        .push(arc_pts.clone());
                }
            }

            for (valid_idx, (_poly_id, mut segs)) in poly_arcs.into_iter().enumerate() {
                let mut rings = Vec::new();
                while !segs.is_empty() {
                    let mut ring = segs.remove(0);
                    let mut changed = true;
                    while changed && !segs.is_empty() {
                        changed = false;
                        for idx in 0..segs.len() {
                            let first = *ring.first().unwrap();
                            let last = *ring.last().unwrap();
                            let s_first = *segs[idx].first().unwrap();
                            let s_last = *segs[idx].last().unwrap();
                            if points_close(last, s_first) {
                                let seg = segs.remove(idx);
                                ring.extend_from_slice(&seg[1..]);
                                changed = true;
                                break;
                            } else if points_close(last, s_last) {
                                let mut seg = segs.remove(idx);
                                seg.reverse();
                                ring.extend_from_slice(&seg[1..]);
                                changed = true;
                                break;
                            } else if points_close(first, s_last) {
                                let mut seg = segs.remove(idx);
                                seg.pop();
                                seg.extend(ring);
                                ring = seg;
                                changed = true;
                                break;
                            } else if points_close(first, s_first) {
                                let mut seg = segs.remove(idx);
                                seg.reverse();
                                seg.pop();
                                seg.extend(ring);
                                ring = seg;
                                changed = true;
                                break;
                            }
                        }
                    }
                    if let (Some(&f), Some(&l)) = (ring.first(), ring.last()) {
                        if !points_close(f, l) {
                            ring.push(f);
                        }
                    }
                    if ring.len() >= 4 {
                        rings.push(ring);
                    }
                }
                if rings.is_empty() {
                    continue;
                }
                let mut props = attr_rows.get(valid_idx).cloned().unwrap_or_default();
                props
                    .entry("ID".to_owned())
                    .or_insert_with(|| Value::from(valid_idx as i64));
                features.push(ParsedMapGisFeature {
                    id: valid_idx,
                    coords: rings,
                    properties: props,
                });
            }
        }
    }

    if fields.is_empty() {
        fields.push(("ID".to_owned(), "int".to_owned(), 10));
    }

    let bbox = compute_features_bbox(&features);
    Ok(ParsedMapGisDataset {
        kind,
        format_variant: format!("MapGIS_6x_Binary_{}", kind.as_str()),
        crs_proj4,
        crs_wkt,
        coordinate_scale: scale,
        fields,
        features,
        bbox,
    })
}

fn points_close(a: [f64; 2], b: [f64; 2]) -> bool {
    (a[0] - b[0]).abs() <= 1e-6 && (a[1] - b[1]).abs() <= 1e-6
}

fn compute_features_bbox(features: &[ParsedMapGisFeature]) -> Option<[f64; 4]> {
    let mut min_x = f64::INFINITY;
    let mut min_y = f64::INFINITY;
    let mut max_x = f64::NEG_INFINITY;
    let mut max_y = f64::NEG_INFINITY;
    let mut any = false;
    for feat in features {
        for ring in &feat.coords {
            for &[x, y] in ring {
                if x.is_finite() && y.is_finite() {
                    min_x = min_x.min(x);
                    min_y = min_y.min(y);
                    max_x = max_x.max(x);
                    max_y = max_y.max(y);
                    any = true;
                }
            }
        }
    }
    any.then_some([min_x, min_y, max_x, max_y])
}

fn parse_mapgis_clear_text(
    bytes: &[u8],
    scale_override: Option<f64>,
) -> Result<ParsedMapGisDataset, NativeGisError> {
    let text = String::from_utf8_lossy(bytes);
    let mut lines = text
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty() && !l.starts_with("//"));
    let header = lines.next().unwrap_or_default().to_ascii_uppercase();
    let scale = scale_override.unwrap_or(1.0);
    let kind = if header.contains("WPNT") || header.contains("POINT") {
        MapGisShapeKind::Point
    } else if header.contains("WPOL") || header.contains("POLYGON") {
        MapGisShapeKind::Polygon
    } else {
        MapGisShapeKind::Line
    };
    let mut features = Vec::new();
    let mut current_pts = Vec::new();
    for line in lines {
        let parts: Vec<&str> = line
            .split(|c: char| c == ',' || c.is_ascii_whitespace())
            .filter(|s| !s.is_empty())
            .collect();
        if parts.len() >= 2 {
            if let (Ok(x), Ok(y)) = (parts[0].parse::<f64>(), parts[1].parse::<f64>()) {
                if kind == MapGisShapeKind::Point {
                    let id = features.len();
                    let mut props = Map::new();
                    props.insert("ID".to_owned(), Value::from(id as i64));
                    features.push(ParsedMapGisFeature {
                        id,
                        coords: vec![vec![[x * scale, y * scale]]],
                        properties: props,
                    });
                } else {
                    current_pts.push([x * scale, y * scale]);
                }
                continue;
            }
        }
        if line.eq_ignore_ascii_case("END") && !current_pts.is_empty() {
            let id = features.len();
            let mut ring = std::mem::take(&mut current_pts);
            if kind == MapGisShapeKind::Polygon {
                if let (Some(&f), Some(&l)) = (ring.first(), ring.last()) {
                    if !points_close(f, l) {
                        ring.push(f);
                    }
                }
            }
            let mut props = Map::new();
            props.insert("ID".to_owned(), Value::from(id as i64));
            features.push(ParsedMapGisFeature {
                id,
                coords: vec![ring],
                properties: props,
            });
        }
    }
    if !current_pts.is_empty() {
        let id = features.len();
        if kind == MapGisShapeKind::Polygon {
            if let (Some(&f), Some(&l)) = (current_pts.first(), current_pts.last()) {
                if !points_close(f, l) {
                    current_pts.push(f);
                }
            }
        }
        let mut props = Map::new();
        props.insert("ID".to_owned(), Value::from(id as i64));
        features.push(ParsedMapGisFeature {
            id,
            coords: vec![current_pts],
            properties: props,
        });
    }
    let bbox = compute_features_bbox(&features);
    Ok(ParsedMapGisDataset {
        kind,
        format_variant: format!("MapGIS_6x_ClearText_{}", kind.as_str()),
        crs_proj4: Some("+proj=longlat +datum=WGS84 +no_defs".to_owned()),
        crs_wkt: Some(r#"GEOGCS["WGS 84",DATUM["WGS_1984",SPHEROID["WGS 84",6378137,298.257223563]],PRIMEM["Greenwich",0],UNIT["degree",0.0174532925199433],AUTHORITY["EPSG","4326"]]"#.to_owned()),
        coordinate_scale: scale,
        fields: vec![("ID".to_owned(), "int".to_owned(), 10)],
        features,
        bbox,
    })
}

pub fn inspect_mapgis_file(path: &Path) -> Result<Value, NativeGisError> {
    let bytes = fs::read(path).map_err(|_| {
        error(
            NativeGisErrorCode::AoiFormatUnsupported,
            "无法读取 MapGIS 源文件",
        )
    })?;
    let parsed = parse_mapgis_bytes(&bytes, None)?;
    Ok(json!({
        "ok": true,
        "path": path.to_string_lossy(),
        "size_bytes": bytes.len(),
        "shape_type": parsed.kind.as_str(),
        "geojson_geometry_type": parsed.kind.geojson_type(),
        "format_variant": parsed.format_variant,
        "feature_count": parsed.features.len(),
        "coordinate_scale": parsed.coordinate_scale,
        "crs_proj4": parsed.crs_proj4,
        "crs_wkt": parsed.crs_wkt,
        "bbox": parsed.bbox,
        "fields": parsed.fields.iter().map(|(name, ftype, len)| json!({
            "name": name,
            "type": ftype,
            "length": len,
        })).collect::<Vec<_>>(),
    }))
}

pub fn dataset_to_geojson(parsed: &ParsedMapGisDataset) -> Value {
    let features: Vec<Value> = parsed
        .features
        .iter()
        .map(|f| {
            let geometry = match parsed.kind {
                MapGisShapeKind::Point => {
                    let pt = f
                        .coords
                        .first()
                        .and_then(|r| r.first())
                        .copied()
                        .unwrap_or([0.0, 0.0]);
                    json!({
                        "type": "Point",
                        "coordinates": [pt[0], pt[1]],
                    })
                }
                MapGisShapeKind::Line => {
                    let line = f.coords.first().cloned().unwrap_or_default();
                    json!({
                        "type": "LineString",
                        "coordinates": line,
                    })
                }
                MapGisShapeKind::Polygon => json!({
                    "type": "Polygon",
                    "coordinates": f.coords,
                }),
            };
            json!({
                "type": "Feature",
                "id": f.id,
                "properties": f.properties,
                "geometry": geometry,
            })
        })
        .collect();
    json!({
        "type": "FeatureCollection",
        "name": parsed.format_variant,
        "bbox": parsed.bbox,
        "features": features,
    })
}

pub(crate) fn write_shapefile_bundle(
    parsed: &ParsedMapGisDataset,
    shp_path: &Path,
) -> Result<Vec<String>, NativeGisError> {
    let shp_type: i32 = match parsed.kind {
        MapGisShapeKind::Point => 1,
        MapGisShapeKind::Line => 3,
        MapGisShapeKind::Polygon => 5,
    };
    let bbox = parsed.bbox.unwrap_or([0.0, 0.0, 0.0, 0.0]);
    let mut shp_records: Vec<Vec<u8>> = Vec::with_capacity(parsed.features.len());
    let mut shx_entries: Vec<(i32, i32)> = Vec::with_capacity(parsed.features.len());
    let mut word_offset: i32 = 50; // 100-byte header = 50 16-bit words

    for (idx, feat) in parsed.features.iter().enumerate() {
        let mut content = Vec::new();
        content.extend_from_slice(&shp_type.to_le_bytes());
        match parsed.kind {
            MapGisShapeKind::Point => {
                let pt = feat
                    .coords
                    .first()
                    .and_then(|r| r.first())
                    .copied()
                    .unwrap_or([0.0, 0.0]);
                content.extend_from_slice(&pt[0].to_le_bytes());
                content.extend_from_slice(&pt[1].to_le_bytes());
            }
            MapGisShapeKind::Line | MapGisShapeKind::Polygon => {
                let f_bbox = compute_features_bbox(std::slice::from_ref(feat))
                    .unwrap_or([0.0, 0.0, 0.0, 0.0]);
                for b in f_bbox {
                    content.extend_from_slice(&b.to_le_bytes());
                }
                let num_parts = feat.coords.len() as i32;
                let num_points: i32 = feat.coords.iter().map(|r| r.len() as i32).sum();
                content.extend_from_slice(&num_parts.to_le_bytes());
                content.extend_from_slice(&num_points.to_le_bytes());
                let mut part_start: i32 = 0;
                for ring in &feat.coords {
                    content.extend_from_slice(&part_start.to_le_bytes());
                    part_start += ring.len() as i32;
                }
                for ring in &feat.coords {
                    for &[x, y] in ring {
                        content.extend_from_slice(&x.to_le_bytes());
                        content.extend_from_slice(&y.to_le_bytes());
                    }
                }
            }
        }
        let content_words = (content.len() / 2) as i32;
        shx_entries.push((word_offset, content_words));
        word_offset += 4 + content_words;

        let mut record = Vec::with_capacity(8 + content.len());
        record.extend_from_slice(&((idx as i32) + 1).to_be_bytes());
        record.extend_from_slice(&content_words.to_be_bytes());
        record.extend_from_slice(&content);
        shp_records.push(record);
    }

    let make_main_header = |file_length_words: i32| -> Vec<u8> {
        let mut h = vec![0u8; 100];
        h[0..4].copy_from_slice(&9994i32.to_be_bytes());
        h[24..28].copy_from_slice(&file_length_words.to_be_bytes());
        h[28..32].copy_from_slice(&1000i32.to_le_bytes());
        h[32..36].copy_from_slice(&shp_type.to_le_bytes());
        h[36..44].copy_from_slice(&bbox[0].to_le_bytes());
        h[44..52].copy_from_slice(&bbox[1].to_le_bytes());
        h[52..60].copy_from_slice(&bbox[2].to_le_bytes());
        h[60..68].copy_from_slice(&bbox[3].to_le_bytes());
        h
    };

    let mut shp_bytes = make_main_header(word_offset);
    for rec in &shp_records {
        shp_bytes.extend_from_slice(rec);
    }
    let shx_length_words = 50 + (shx_entries.len() as i32) * 4;
    let mut shx_bytes = make_main_header(shx_length_words);
    for (off, len) in &shx_entries {
        shx_bytes.extend_from_slice(&off.to_be_bytes());
        shx_bytes.extend_from_slice(&len.to_be_bytes());
    }

    // Minimal standard dBASE III (.dbf) with ID (N, 10, 0) + NAME (C, 32)
    let num_records = parsed.features.len() as u32;
    let num_fields = 2usize;
    let header_len = (32 + num_fields * 32 + 1) as u16;
    let record_len = (1 + 10 + 32) as u16;
    let mut dbf = Vec::with_capacity(
        header_len as usize + (num_records as usize) * (record_len as usize) + 1,
    );
    dbf.push(0x03); // dBASE III
    dbf.extend_from_slice(&[26, 11, 7]); // YY MM DD
    dbf.extend_from_slice(&num_records.to_le_bytes());
    dbf.extend_from_slice(&header_len.to_le_bytes());
    dbf.extend_from_slice(&record_len.to_le_bytes());
    dbf.extend_from_slice(&[0u8; 20]);

    let mut f1 = [0u8; 32];
    f1[0..2].copy_from_slice(b"ID");
    f1[11] = b'N';
    f1[16] = 10;
    dbf.extend_from_slice(&f1);

    let mut f2 = [0u8; 32];
    f2[0..4].copy_from_slice(b"NAME");
    f2[11] = b'C';
    f2[16] = 32;
    dbf.extend_from_slice(&f2);
    dbf.push(0x0D); // Header terminator

    for feat in &parsed.features {
        dbf.push(0x20); // Active record
        let id_str = format!("{:>10}", feat.id);
        dbf.extend_from_slice(&id_str.as_bytes()[..10]);
        let name_val = feat
            .properties
            .get("NAME")
            .or_else(|| feat.properties.get("StrText"))
            .and_then(Value::as_str)
            .unwrap_or("");
        let mut name_buf = [0x20u8; 32];
        let copy_len = name_val.len().min(32);
        name_buf[..copy_len].copy_from_slice(&name_val.as_bytes()[..copy_len]);
        dbf.extend_from_slice(&name_buf);
    }
    dbf.push(0x1A); // EOF

    let shx_path = shp_path.with_extension("shx");
    let dbf_path = shp_path.with_extension("dbf");
    let prj_path = shp_path.with_extension("prj");
    fs::write(shp_path, &shp_bytes).map_err(|_| {
        error(
            NativeGisErrorCode::AoiFormatUnsupported,
            "无法写入 Shapefile .shp 文件",
        )
    })?;
    fs::write(&shx_path, &shx_bytes).map_err(|_| {
        error(
            NativeGisErrorCode::AoiFormatUnsupported,
            "无法写入 Shapefile .shx 文件",
        )
    })?;
    fs::write(&dbf_path, &dbf).map_err(|_| {
        error(
            NativeGisErrorCode::AoiFormatUnsupported,
            "无法写入 Shapefile .dbf 文件",
        )
    })?;
    let mut written_files = vec![
        shp_path.to_string_lossy().to_string(),
        shx_path.to_string_lossy().to_string(),
        dbf_path.to_string_lossy().to_string(),
    ];
    let wkt = parsed.crs_wkt.as_deref().unwrap_or(
        r#"GEOGCS["WGS 84",DATUM["WGS_1984",SPHEROID["WGS 84",6378137,298.257223563]],PRIMEM["Greenwich",0],UNIT["degree",0.0174532925199433],AUTHORITY["EPSG","4326"]]"#,
    );
    if fs::write(&prj_path, wkt.as_bytes()).is_ok() {
        written_files.push(prj_path.to_string_lossy().to_string());
    }
    Ok(written_files)
}

pub fn convert_mapgis_file(
    input: &Path,
    output: &Path,
    scale_override: Option<f64>,
) -> Result<Value, NativeGisError> {
    let bytes = fs::read(input).map_err(|_| {
        error(
            NativeGisErrorCode::AoiFormatUnsupported,
            "无法读取 MapGIS 源文件",
        )
    })?;
    let parsed = parse_mapgis_bytes(&bytes, scale_override)?;
    if let Some(parent) = output.parent() {
        fs::create_dir_all(parent).map_err(|_| {
            error(
                NativeGisErrorCode::AoiFormatUnsupported,
                "无法创建 MapGIS 转换输出目录",
            )
        })?;
    }
    let ext = output
        .extension()
        .and_then(|s| s.to_str())
        .unwrap_or("geojson")
        .to_ascii_lowercase();
    let geojson_val = dataset_to_geojson(&parsed);
    let mut written_files = Vec::new();
    if ext == "shp" {
        written_files = write_shapefile_bundle(&parsed, output)?;
        let companion_geojson = output.with_extension("geojson");
        if let Ok(serialized) = serde_json::to_vec_pretty(&geojson_val) {
            if fs::write(&companion_geojson, serialized).is_ok() {
                written_files.push(companion_geojson.to_string_lossy().to_string());
            }
        }
    } else {
        let serialized = serde_json::to_vec_pretty(&geojson_val).map_err(|_| {
            error(
                NativeGisErrorCode::AoiFormatUnsupported,
                "序列化 GeoJSON 失败",
            )
        })?;
        fs::write(output, &serialized).map_err(|_| {
            error(
                NativeGisErrorCode::AoiFormatUnsupported,
                "无法写入转换后的 GeoJSON 文件",
            )
        })?;
        written_files.push(output.to_string_lossy().to_string());
    }
    let bytes_written = fs::metadata(output).map(|m| m.len()).unwrap_or(0);
    Ok(json!({
        "ok": true,
        "executed": true,
        "input_path": input.to_string_lossy(),
        "output_path": output.to_string_lossy(),
        "written_files": written_files,
        "shape_type": parsed.kind.as_str(),
        "format_variant": parsed.format_variant,
        "feature_count": parsed.features.len(),
        "bbox": parsed.bbox,
        "crs_proj4": parsed.crs_proj4,
        "bytes_written": bytes_written,
        "geojson_preview": geojson_val,
    }))
}

/// Build a valid MapGIS 6.x binary `.wt` (Point) or `.wl` (Line) payload for
/// automated round-trip testing and verification.
pub fn build_synthetic_mapgis_binary_wl(lines: &[Vec<[f64; 2]>]) -> Vec<u8> {
    let mut buf = vec![0u8; 200];
    buf[0..8].copy_from_slice(b"WMAP`D21");
    // CRS metadata at 109 (proj_type=0 geographic, ellipsoid=7 WGS84)
    buf[109] = 0;
    buf[110] = 7;
    buf[143..151].copy_from_slice(&1.0f64.to_le_bytes());

    let data_start = buf.len() as i32;
    buf[12..16].copy_from_slice(&data_start.to_le_bytes());
    // Reserve 10 section headers (100 bytes)
    buf.resize(data_start as usize + 100, 0);

    let ln_start = buf.len() as i32;
    // Dummy record 0 (57 bytes)
    buf.extend_from_slice(&[0u8; 57]);
    let mut xy_pool = Vec::new();
    for line in lines {
        let mut rec = [0u8; 57];
        rec[0] = 1; // active_flag
        rec[1..3].copy_from_slice(&2u16.to_le_bytes());
        let node_count = line.len() as i32;
        let node_offset = xy_pool.len() as i32;
        rec[10..14].copy_from_slice(&node_count.to_le_bytes());
        rec[14..18].copy_from_slice(&node_offset.to_le_bytes());
        rec[22..26].copy_from_slice(&1i32.to_le_bytes()); // line_type
        rec[26..30].copy_from_slice(&6i32.to_le_bytes()); // line_color
        rec[30..34].copy_from_slice(&0.5f32.to_le_bytes()); // line_width
        for &[x, y] in line {
            xy_pool.extend_from_slice(&x.to_le_bytes());
            xy_pool.extend_from_slice(&y.to_le_bytes());
        }
        buf.extend_from_slice(&rec);
    }
    let ln_vol = (buf.len() as i32) - ln_start;
    let xy_start = buf.len() as i32;
    buf.extend_from_slice(&xy_pool);
    let xy_vol = xy_pool.len() as i32;

    let h0 = data_start as usize;
    buf[h0..h0 + 4].copy_from_slice(&ln_start.to_le_bytes());
    buf[h0 + 4..h0 + 8].copy_from_slice(&ln_vol.to_le_bytes());
    let h1 = h0 + 10;
    buf[h1..h1 + 4].copy_from_slice(&xy_start.to_le_bytes());
    buf[h1 + 4..h1 + 8].copy_from_slice(&xy_vol.to_le_bytes());
    buf
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mapgis_binary_wl_inspect_and_convert_to_shp_and_geojson_works() {
        let sample_lines = vec![
            vec![[117.20, 31.80], [117.25, 31.82], [117.30, 31.85]],
            vec![[117.10, 31.75], [117.15, 31.78]],
        ];
        let wl_bytes = build_synthetic_mapgis_binary_wl(&sample_lines);
        let dir =
            std::env::temp_dir().join(format!("insar-gis-mapgis-test-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        let wl_path = dir.join("hefei_faults.wl");
        fs::write(&wl_path, &wl_bytes).unwrap();

        let info = inspect_mapgis_file(&wl_path).expect("inspect synthetic .wl");
        assert_eq!(info["shape_type"], "LINE");
        assert_eq!(info["feature_count"], 2);

        let shp_out = dir.join("hefei_faults.shp");
        let converted = convert_mapgis_file(&wl_path, &shp_out, None).expect("convert .wl to .shp");
        assert_eq!(converted["feature_count"], 2);
        assert!(shp_out.is_file());
        assert!(shp_out.with_extension("shx").is_file());
        assert!(shp_out.with_extension("dbf").is_file());
        assert!(shp_out.with_extension("prj").is_file());
        assert!(shp_out.with_extension("geojson").is_file());

        let _ = fs::remove_dir_all(&dir);
    }
}
