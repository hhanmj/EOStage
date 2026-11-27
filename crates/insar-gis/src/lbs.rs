use super::{NativeGisError, NativeGisErrorCode};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

const PI: f64 = std::f64::consts::PI;
const X_PI: f64 = std::f64::consts::PI * 3000.0 / 180.0;
const KRASSOVSKY_A: f64 = 6378245.0;
const KRASSOVSKY_EE: f64 = 0.006693421622965943;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum LbsCoordSys {
    Wgs84,
    Gcj02,
    Bd09,
    Cgcs2000,
}

impl LbsCoordSys {
    pub fn parse(raw: &str) -> Result<Self, NativeGisError> {
        let norm = raw
            .trim()
            .to_ascii_lowercase()
            .replace(['-', '_', ':', ' '], "");
        match norm.as_str() {
            "wgs84" | "epsg4326" | "4326" | "gps" => Ok(Self::Wgs84),
            "gcj02" | "amap" | "gaode" | "tencent" | "mars" => Ok(Self::Gcj02),
            "bd09" | "bd09ll" | "baidu" => Ok(Self::Bd09),
            "cgcs2000" | "epsg4490" | "4490" | "tianditu" => Ok(Self::Cgcs2000),
            _ => Err(NativeGisError::contract(
                NativeGisErrorCode::AoiFormatUnsupported,
                format!("不支持的国内/LBS坐标系标识: {raw} (支持 wgs84 / gcj02 / bd09 / cgcs2000)"),
            )),
        }
    }

    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Wgs84 => "wgs84",
            Self::Gcj02 => "gcj02",
            Self::Bd09 => "bd09",
            Self::Cgcs2000 => "cgcs2000",
        }
    }
}

pub fn is_out_of_china(lng: f64, lat: f64) -> bool {
    !(72.004..=137.8347).contains(&lng) || !(0.8293..=55.8271).contains(&lat)
}

fn transform_lat(lng: f64, lat: f64) -> f64 {
    let mut ret =
        -100.0 + 2.0 * lng + 3.0 * lat + 0.2 * lat * lat + 0.1 * lng * lat + 0.2 * lng.abs().sqrt();
    ret += (20.0 * (6.0 * lng * PI).sin() + 20.0 * (2.0 * lng * PI).sin()) * 2.0 / 3.0;
    ret += (20.0 * (lat * PI).sin() + 40.0 * (lat / 3.0 * PI).sin()) * 2.0 / 3.0;
    ret += (160.0 * (lat / 12.0 * PI).sin() + 320.0 * (lat * PI / 30.0).sin()) * 2.0 / 3.0;
    ret
}

fn transform_lng(lng: f64, lat: f64) -> f64 {
    let mut ret =
        300.0 + lng + 2.0 * lat + 0.1 * lng * lng + 0.1 * lng * lat + 0.1 * lng.abs().sqrt();
    ret += (20.0 * (6.0 * lng * PI).sin() + 20.0 * (2.0 * lng * PI).sin()) * 2.0 / 3.0;
    ret += (20.0 * (lng * PI).sin() + 40.0 * (lng / 3.0 * PI).sin()) * 2.0 / 3.0;
    ret += (150.0 * (lng / 12.0 * PI).sin() + 300.0 * (lng / 30.0 * PI).sin()) * 2.0 / 3.0;
    ret
}

fn delta(lng: f64, lat: f64) -> (f64, f64) {
    let dlat = transform_lat(lng - 105.0, lat - 35.0);
    let dlng = transform_lng(lng - 105.0, lat - 35.0);
    let radlat = lat / 180.0 * PI;
    let mut magic = radlat.sin();
    magic = 1.0 - KRASSOVSKY_EE * magic * magic;
    let sqrtmagic = magic.sqrt();
    let dlat = (dlat * 180.0) / ((KRASSOVSKY_A * (1.0 - KRASSOVSKY_EE)) / (magic * sqrtmagic) * PI);
    let dlng = (dlng * 180.0) / (KRASSOVSKY_A / sqrtmagic * radlat.cos() * PI);
    (dlng, dlat)
}

pub fn wgs84_to_gcj02(lng: f64, lat: f64) -> (f64, f64) {
    if is_out_of_china(lng, lat) {
        return (lng, lat);
    }
    let (dlng, dlat) = delta(lng, lat);
    (lng + dlng, lat + dlat)
}

/// High-precision iterative fixed-point inversion (exact to < 1e-9 deg).
pub fn gcj02_to_wgs84(lng: f64, lat: f64) -> (f64, f64) {
    if is_out_of_china(lng, lat) {
        return (lng, lat);
    }
    let (mut wgs_lng, mut wgs_lat) = (lng, lat);
    for _ in 0..6 {
        let (est_gcj_lng, est_gcj_lat) = wgs84_to_gcj02(wgs_lng, wgs_lat);
        let dlng = est_gcj_lng - lng;
        let dlat = est_gcj_lat - lat;
        wgs_lng -= dlng;
        wgs_lat -= dlat;
        if dlng.abs() < 1e-10 && dlat.abs() < 1e-10 {
            break;
        }
    }
    (wgs_lng, wgs_lat)
}

pub fn gcj02_to_bd09(lng: f64, lat: f64) -> (f64, f64) {
    let z = (lng * lng + lat * lat).sqrt() + 0.00002 * (lat * X_PI).sin();
    let theta = lat.atan2(lng) + 0.000003 * (lng * X_PI).cos();
    let bd_lng = z * theta.cos() + 0.0065;
    let bd_lat = z * theta.sin() + 0.006;
    (bd_lng, bd_lat)
}

pub fn bd09_to_gcj02(lng: f64, lat: f64) -> (f64, f64) {
    let x = lng - 0.0065;
    let y = lat - 0.006;
    let z = (x * x + y * y).sqrt() - 0.00002 * (y * X_PI).sin();
    let theta = y.atan2(x) - 0.000003 * (x * X_PI).cos();
    let mut gcj_lng = z * theta.cos();
    let mut gcj_lat = z * theta.sin();
    for _ in 0..5 {
        let (est_bd_lng, est_bd_lat) = gcj02_to_bd09(gcj_lng, gcj_lat);
        let dlng = est_bd_lng - lng;
        let dlat = est_bd_lat - lat;
        gcj_lng -= dlng;
        gcj_lat -= dlat;
        if dlng.abs() < 1e-11 && dlat.abs() < 1e-11 {
            break;
        }
    }
    (gcj_lng, gcj_lat)
}

pub fn transform_coord_pair(lng: f64, lat: f64, from: LbsCoordSys, to: LbsCoordSys) -> (f64, f64) {
    if from == to {
        return (lng, lat);
    }
    // Normalize `from` to WGS84/CGCS2000 (equivalent within sub-millimeter horizontal datum)
    let (wgs_lng, wgs_lat) = match from {
        LbsCoordSys::Wgs84 | LbsCoordSys::Cgcs2000 => (lng, lat),
        LbsCoordSys::Gcj02 => gcj02_to_wgs84(lng, lat),
        LbsCoordSys::Bd09 => {
            let (g_lng, g_lat) = bd09_to_gcj02(lng, lat);
            gcj02_to_wgs84(g_lng, g_lat)
        }
    };
    match to {
        LbsCoordSys::Wgs84 | LbsCoordSys::Cgcs2000 => (wgs_lng, wgs_lat),
        LbsCoordSys::Gcj02 => wgs84_to_gcj02(wgs_lng, wgs_lat),
        LbsCoordSys::Bd09 => {
            let (g_lng, g_lat) = wgs84_to_gcj02(wgs_lng, wgs_lat);
            gcj02_to_bd09(g_lng, g_lat)
        }
    }
}

pub fn transform_lbs_coordinates(
    from_raw: &str,
    to_raw: &str,
    points: &[[f64; 2]],
) -> Result<Value, NativeGisError> {
    let from = LbsCoordSys::parse(from_raw)?;
    let to = LbsCoordSys::parse(to_raw)?;
    if points.is_empty() {
        return Err(NativeGisError::contract(
            NativeGisErrorCode::AoiGeometryInvalid,
            "至少需要提供一组经纬度坐标 [lng, lat]",
        ));
    }
    let mut results = Vec::with_capacity(points.len());
    for &[lng, lat] in points {
        if !lng.is_finite()
            || !lat.is_finite()
            || !(-180.0..=180.0).contains(&lng)
            || !(-90.0..=90.0).contains(&lat)
        {
            return Err(NativeGisError::contract(
                NativeGisErrorCode::AoiGeometryInvalid,
                format!("无效经纬度坐标: [{lng}, {lat}]"),
            ));
        }
        let (out_lng, out_lat) = transform_coord_pair(lng, lat, from, to);
        results.push(json!({
            "input": [lng, lat],
            "output": [(out_lng * 1e8).round() / 1e8, (out_lat * 1e8).round() / 1e8],
            "delta_meters_approx": [
                ((out_lng - lng) * 111_320.0 * lat.to_radians().cos() * 100.0).round() / 100.0,
                ((out_lat - lat) * 111_320.0 * 100.0).round() / 100.0,
            ],
            "in_china_bounds": !is_out_of_china(lng, lat),
        }));
    }
    Ok(json!({
        "ok": true,
        "from": from.as_str(),
        "to": to.as_str(),
        "count": results.len(),
        "results": results,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lbs_coord_round_trip_wgs84_gcj02_bd09_is_exact() {
        // Hefei center (approx 117.2272, 31.8206)
        let (lng0, lat0) = (117.227239, 31.820586);
        let (gcj_lng, gcj_lat) =
            transform_coord_pair(lng0, lat0, LbsCoordSys::Wgs84, LbsCoordSys::Gcj02);
        assert!((gcj_lng - lng0).abs() > 0.001, "Hefei must shift in GCJ02");
        let (bd_lng, bd_lat) =
            transform_coord_pair(gcj_lng, gcj_lat, LbsCoordSys::Gcj02, LbsCoordSys::Bd09);
        let (back_lng, back_lat) =
            transform_coord_pair(bd_lng, bd_lat, LbsCoordSys::Bd09, LbsCoordSys::Wgs84);
        assert!(
            (back_lng - lng0).abs() < 1e-7,
            "round-trip lng error too large"
        );
        assert!(
            (back_lat - lat0).abs() < 1e-7,
            "round-trip lat error too large"
        );
    }
}
