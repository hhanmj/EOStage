//! Loss-averse STAC SAR 1.3.2 / InSAR proposal field projection.
//! Only declared properties are read; the source item and asset metadata remain
//! authoritative. No displacement-unit or viewing-sign convention is inferred.
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use std::collections::HashSet;

pub const SAR_STAC_EXTENSION: &str = "https://stac-extensions.github.io/sar/v1.3.2/schema.json";
pub const INSAR_STAC_PROPOSAL: &str = "https://stac-extensions.github.io/insar/v1.0.0/schema.json";

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SarStacScientificMetadata {
    pub frequency_band: Option<String>,
    pub polarizations: Option<Vec<String>>,
    /// Antenna left/right relative to flight. NOT ascending/descending orbit.
    pub observation_direction: Option<String>,
    /// Ascending/descending orbit state from the satellite extension.
    pub orbit_state: Option<String>,
    /// Product extension; intentionally no fallback to deprecated sar:product_type.
    pub product_type: Option<String>,
    pub center_frequency_ghz: Option<f64>,
    pub range_pixel_spacing_m: Option<f64>,
    pub azimuth_pixel_spacing_m: Option<f64>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct InsarPairScientificMetadata {
    pub reference_datetime_utc: DateTime<Utc>,
    pub secondary_datetime_utc: DateTime<Utc>,
    /// Raw numeric proposal value; do not assume conversion/units without product review.
    pub temporal_baseline_source_value: Option<f64>,
    pub perpendicular_baseline_source_value: Option<f64>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct StacScientificMetadata {
    pub sar: SarStacScientificMetadata,
    /// Present only if BOTH acquisition timestamps are supplied and well-formed.
    /// Time-series products do not necessarily have one single interferometric pair.
    pub insar_pair: Option<InsarPairScientificMetadata>,
}

fn optional_string(p: &Map<String, Value>, key: &str) -> Result<Option<String>, String> {
    match p.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(s)) if !s.trim().is_empty() => Ok(Some(s.to_owned())),
        _ => Err(format!(
            "invalid STAC field {key}: expected non-empty string"
        )),
    }
}

fn optional_nonnegative_finite(p: &Map<String, Value>, key: &str) -> Result<Option<f64>, String> {
    match p.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::Number(n)) => match n.as_f64() {
            Some(v) if v.is_finite() && v >= 0.0 => Ok(Some(v)),
            _ => Err(format!(
                "invalid STAC field {key}: expected finite nonnegative number"
            )),
        },
        _ => Err(format!("invalid STAC field {key}: expected number")),
    }
}

fn optional_signed_finite(p: &Map<String, Value>, key: &str) -> Result<Option<f64>, String> {
    match p.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::Number(n)) => match n.as_f64() {
            Some(v) if v.is_finite() => Ok(Some(v)),
            _ => Err(format!("invalid STAC field {key}: expected finite number")),
        },
        _ => Err(format!("invalid STAC field {key}: expected number")),
    }
}

fn utc_datetime(p: &Map<String, Value>, key: &str) -> Result<Option<DateTime<Utc>>, String> {
    let Some(s) = optional_string(p, key)? else {
        return Ok(None);
    };
    if !s.ends_with('Z') {
        return Err(format!(
            "invalid STAC field {key}: UTC Z timestamp required"
        ));
    }
    DateTime::parse_from_rfc3339(&s)
        .map(|value| Some(value.with_timezone(&Utc)))
        .map_err(|_| format!("invalid STAC field {key}: RFC3339 timestamp required"))
}

/// Parse only a STAC Item's 'properties' object; do not pass an untrusted asset
/// object or arbitrary STAC JSON root into this projection.
pub fn project_stac_scientific_metadata(
    properties: &Map<String, Value>,
) -> Result<StacScientificMetadata, String> {
    let observation_direction = optional_string(properties, "sar:observation_direction")?;
    if let Some(direction) = observation_direction.as_deref() {
        if direction != "left" && direction != "right" {
            return Err("invalid sar:observation_direction".into());
        }
    }
    let orbit_state = optional_string(properties, "sat:orbit_state")?;
    if let Some(state) = orbit_state.as_deref() {
        if state != "ascending" && state != "descending" && state != "geostationary" {
            return Err("invalid sat:orbit_state".into());
        }
    }

    // v1.3.2 enumerates these common SAR frequency-band identifiers.
    let frequency_band = optional_string(properties, "sar:frequency_band")?;
    if let Some(band) = frequency_band.as_deref() {
        if !["P", "L", "S", "C", "X", "Ku", "K", "Ka"].contains(&band) {
            return Err("invalid sar:frequency_band".into());
        }
    }
    let center_frequency_ghz = optional_nonnegative_finite(properties, "sar:center_frequency")?;
    if center_frequency_ghz == Some(0.0) {
        return Err("invalid sar:center_frequency: must be strictly positive".into());
    }

    let polarizations = match properties.get("sar:polarizations") {
        None | Some(Value::Null) => None,
        Some(Value::Array(values)) if !values.is_empty() => {
            let allowed = ["HH", "VV", "HV", "VH", "LH", "LV", "RH", "RV", "CH", "CV"];
            let mut seen = HashSet::new();
            let mut parsed = Vec::new();
            for entry in values {
                let p = entry
                    .as_str()
                    .filter(|p| allowed.contains(p) && seen.insert(*p))
                    .ok_or("invalid or repeated sar:polarizations entry")?;
                parsed.push(p.to_owned());
            }
            Some(parsed)
        }
        _ => return Err("invalid sar:polarizations: non-empty string array expected".into()),
    };

    let reference = utc_datetime(properties, "insar:reference_datetime")?;
    let secondary = utc_datetime(properties, "insar:secondary_datetime")?;
    if reference.is_some() != secondary.is_some() {
        return Err("insar pair requires both reference and secondary datetimes".into());
    }
    let baseline = optional_signed_finite(properties, "insar:perpendicular_baseline")?;
    let temporal = optional_signed_finite(properties, "insar:temporal_baseline")?;
    if (baseline.is_some() || temporal.is_some()) && reference.is_none() {
        return Err("insar baselines require both pair datetimes".into());
    }
    let insar_pair = match (reference, secondary) {
        (Some(reference_datetime_utc), Some(secondary_datetime_utc)) => {
            if secondary_datetime_utc == reference_datetime_utc {
                return Err("insar reference and secondary must be different acquisitions".into());
            }
            Some(InsarPairScientificMetadata {
                reference_datetime_utc,
                secondary_datetime_utc,
                temporal_baseline_source_value: temporal,
                perpendicular_baseline_source_value: baseline,
            })
        }
        _ => None,
    };

    Ok(StacScientificMetadata {
        sar: SarStacScientificMetadata {
            frequency_band,
            polarizations,
            observation_direction,
            orbit_state,
            product_type: optional_string(properties, "product:type")?,
            center_frequency_ghz,
            range_pixel_spacing_m: optional_nonnegative_finite(
                properties,
                "sar:pixel_spacing_range",
            )?,
            azimuth_pixel_spacing_m: optional_nonnegative_finite(
                properties,
                "sar:pixel_spacing_azimuth",
            )?,
        },
        insar_pair,
    })
}
