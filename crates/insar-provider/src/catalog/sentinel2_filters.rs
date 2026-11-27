//! Product-owned platform vocabulary shared by the CDSE and Microsoft adapters.
use serde_json::Value;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Sentinel2Platform {
    pub value: &'static str,
    pub prefix: &'static str,
    pub stac_name: &'static str,
}

const PLATFORMS: [Sentinel2Platform; 3] = [
    Sentinel2Platform {
        value: "sentinel_2a",
        prefix: "S2A_",
        stac_name: "Sentinel-2A",
    },
    Sentinel2Platform {
        value: "sentinel_2b",
        prefix: "S2B_",
        stac_name: "Sentinel-2B",
    },
    Sentinel2Platform {
        value: "sentinel_2c",
        prefix: "S2C_",
        stac_name: "Sentinel-2C",
    },
];

/// Missing means all admitted platforms, preserving existing frozen queries.
/// Explicit empty, duplicate and unknown values are malformed, never "all".
pub(crate) fn decode_sentinel2_spacecraft(
    value: Option<&Value>,
) -> Result<Vec<Sentinel2Platform>, &'static str> {
    let Some(value) = value else {
        return Ok(Vec::new());
    };
    let values = value
        .as_array()
        .filter(|v| !v.is_empty() && v.len() <= PLATFORMS.len())
        .ok_or("spacecraft")?;
    let mut selected = Vec::with_capacity(values.len());
    for value in values {
        let platform = PLATFORMS
            .iter()
            .find(|p| value.as_str() == Some(p.value))
            .ok_or("spacecraft")?;
        if selected.contains(platform) {
            return Err("spacecraft");
        }
        selected.push(*platform);
    }
    selected.sort_by_key(|p| p.value);
    Ok(selected)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn spacecraft_selection_is_closed_and_canonical() {
        assert!(decode_sentinel2_spacecraft(None).unwrap().is_empty());
        let selected =
            decode_sentinel2_spacecraft(Some(&json!(["sentinel_2c", "sentinel_2a"]))).unwrap();
        assert_eq!(
            selected.iter().map(|p| p.prefix).collect::<Vec<_>>(),
            ["S2A_", "S2C_"]
        );
        for invalid in [
            json!([]),
            json!(["sentinel_2d"]),
            json!(["sentinel_2a", "sentinel_2a"]),
            json!("sentinel_2a"),
            json!([null]),
        ] {
            assert!(decode_sentinel2_spacecraft(Some(&invalid)).is_err());
        }
    }
}
