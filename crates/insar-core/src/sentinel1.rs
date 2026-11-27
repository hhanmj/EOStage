use serde::{Deserialize, Serialize};
use serde_json::{json, Map};

use crate::{ContractError, ProviderResult};

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ParsedSentinel1Name {
    pub scene_id: String,
    pub source_url: Option<String>,
    pub platform: String,
    pub beam_mode: String,
    pub product_type: String,
    pub resolution: String,
    pub processing_level: u8,
    pub processing_class: String,
    pub polarization: String,
    pub polarization_channels: Vec<String>,
    pub acquisition_start: String,
    pub acquisition_stop: String,
    pub absolute_orbit: u64,
    pub datatake_id: String,
    pub unique_id: Option<String>,
}

fn parse_error(input: &str) -> ContractError {
    let without_query = input.split('?').next().unwrap_or_default();
    let safe_input = without_query.chars().take(256).collect::<String>();
    let context = Map::from_iter([("input".to_string(), json!(safe_input))]);
    ContractError::new("ASF001", "无法识别 Sentinel-1 影像名称", context)
        .expect("the Sentinel-1 parse error context is safe")
}

fn strip_known_extension(candidate: &str) -> &str {
    let lowercase = candidate.to_ascii_lowercase();
    for extension in [".zip", ".safe"] {
        if lowercase.ends_with(extension) {
            return &candidate[..candidate.len() - extension.len()];
        }
    }
    candidate
}

fn is_exact_ascii(value: &str, length: usize, predicate: impl Fn(u8) -> bool) -> bool {
    value.len() == length && value.bytes().all(predicate)
}

fn is_timestamp(value: &str) -> bool {
    value.len() == 15
        && value.as_bytes().get(8) == Some(&b'T')
        && value
            .bytes()
            .enumerate()
            .all(|(index, byte)| index == 8 || byte.is_ascii_digit())
}

fn is_hex(value: &str, length: usize) -> bool {
    is_exact_ascii(value, length, |byte| byte.is_ascii_hexdigit())
}

fn polarization_channels(code: &str) -> Option<Vec<String>> {
    let channels = match code {
        "SH" => &["HH"][..],
        "SV" => &["VV"][..],
        "DH" => &["HH", "HV"][..],
        "DV" => &["VV", "VH"][..],
        _ => return None,
    };
    Some(
        channels
            .iter()
            .map(|channel| (*channel).to_string())
            .collect(),
    )
}

pub fn parse_sentinel1_identifier(value: &str) -> ProviderResult<ParsedSentinel1Name> {
    let input = value.trim();
    let is_url = input.contains("://");
    let location = input.split('?').next().unwrap_or_default();
    let candidate = location.rsplit(['/', '\\']).next().unwrap_or_default();
    let scene_id = strip_known_extension(candidate);
    let fields = scene_id.split('_').collect::<Vec<_>>();

    let result = (|| {
        let platform = *fields.first()?;
        if platform.len() != 3
            || !platform.starts_with("S1")
            || !matches!(platform.as_bytes()[2], b'A'..=b'D')
        {
            return None;
        }

        let beam_mode = *fields.get(1)?;
        if !matches!(beam_mode, "SM" | "IW" | "EW" | "WV") {
            return None;
        }

        let product_token = *fields.get(2)?;
        if !product_token.is_ascii() {
            return None;
        }
        let (product_type, resolution, class_index) =
            if product_token.len() == 3 && fields.get(3) == Some(&"") {
                (product_token, "_", 4)
            } else if product_token.len() == 4 {
                (&product_token[..3], &product_token[3..], 3)
            } else {
                return None;
            };
        if !matches!(product_type, "SLC" | "GRD" | "RAW" | "OCN")
            || !matches!(resolution, "F" | "H" | "M" | "_")
        {
            return None;
        }

        let classification = *fields.get(class_index)?;
        if classification.len() != 4 || !classification.is_ascii() {
            return None;
        }
        let processing_level = classification[..1].parse::<u8>().ok()?;
        let processing_class = &classification[1..2];
        if !matches!(processing_class, "S" | "A") {
            return None;
        }
        let polarization = &classification[2..];
        let channels = polarization_channels(polarization)?;

        let acquisition_start = *fields.get(class_index + 1)?;
        let acquisition_stop = *fields.get(class_index + 2)?;
        let orbit = *fields.get(class_index + 3)?;
        let datatake = *fields.get(class_index + 4)?;
        if !is_timestamp(acquisition_start)
            || !is_timestamp(acquisition_stop)
            || !is_exact_ascii(orbit, 6, |byte| byte.is_ascii_digit())
            || !is_hex(datatake, 6)
        {
            return None;
        }

        let expected_lengths = [class_index + 5, class_index + 6];
        if !expected_lengths.contains(&fields.len()) {
            return None;
        }
        let unique_id = if fields.len() == class_index + 6 {
            let unique = *fields.get(class_index + 5)?;
            if !is_hex(unique, 4) {
                return None;
            }
            Some(unique.to_ascii_uppercase())
        } else {
            None
        };

        Some(ParsedSentinel1Name {
            scene_id: scene_id.to_string(),
            source_url: is_url.then(|| input.to_string()),
            platform: platform.to_string(),
            beam_mode: beam_mode.to_string(),
            product_type: product_type.to_string(),
            resolution: resolution.to_string(),
            processing_level,
            processing_class: processing_class.to_string(),
            polarization: polarization.to_string(),
            polarization_channels: channels,
            acquisition_start: acquisition_start.to_string(),
            acquisition_stop: acquisition_stop.to_string(),
            absolute_orbit: orbit.parse().ok()?,
            datatake_id: datatake.to_ascii_uppercase(),
            unique_id,
        })
    })();

    result.ok_or_else(|| parse_error(input))
}

pub fn sentinel1_relative_orbit(platform: &str, absolute_orbit: Option<u64>) -> Option<u16> {
    let offset = match platform.trim().to_ascii_uppercase().as_str() {
        "S1A" | "SENTINEL-1A" => 73_i64,
        "S1B" | "SENTINEL-1B" => 27_i64,
        _ => return None,
    };
    let absolute = i64::try_from(absolute_orbit?).ok()?;
    Some(((absolute - offset).rem_euclid(175) + 1) as u16)
}
