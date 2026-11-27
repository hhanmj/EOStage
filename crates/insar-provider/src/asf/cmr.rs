use insar_core::{
    parse_sentinel1_identifier, sentinel1_relative_orbit, AsfRawFeature, AsfSearchQuery,
};
use serde_json::{Map, Number, Value};

use super::query::{asf_datetime, effective_bbox_windows};

const SLC_COLLECTIONS: &[&str] = &[
    "C1214470488-ASF",
    "C1327985661-ASF",
    "C3470873558-ASF",
    "C4175278193-ASF",
];
const GRD_COLLECTIONS: &[&str] = &[
    "C1214470533-ASF",
    "C1214471521-ASF",
    "C1214471197-ASF",
    "C1214470682-ASF",
    "C1214472994-ASF",
    "C1327985645-ASF",
    "C1327985660-ASF",
    "C1327985571-ASF",
    "C1327985740-ASF",
    "C3486566209-ASF",
    "C3486605959-ASF",
    "C3486646217-ASF",
    "C3488402208-ASF",
    "C3488414315-ASF",
    "C3488389367-ASF",
    "C4163054563-ASF",
    "C4174076966-ASF",
    "C4174044457-ASF",
    "C4175200051-ASF",
];

pub fn collection_ids(product_type: &str) -> Option<&'static [&'static str]> {
    match product_type.trim().to_ascii_uppercase().as_str() {
        "SLC" => Some(SLC_COLLECTIONS),
        "GRD" | "GRD_HD" | "GRD_HS" | "GRD_MD" | "GRD_MS" | "GRD_FD" => Some(GRD_COLLECTIONS),
        _ => None,
    }
}

pub fn cmr_page_params(
    query: &AsfSearchQuery,
    page_size: u64,
    page_number: u64,
) -> Option<Vec<(String, String)>> {
    let mut variants = cmr_page_param_variants(query, page_size, page_number)?;
    (variants.len() == 1).then(|| variants.remove(0))
}

pub(crate) fn cmr_page_param_variants(
    query: &AsfSearchQuery,
    page_size: u64,
    page_number: u64,
) -> Option<Vec<Vec<(String, String)>>> {
    let collections = collection_ids(&query.product_type)?;
    let mut params = vec![
        ("page_size".to_string(), page_size.to_string()),
        ("page_num".to_string(), page_number.to_string()),
        ("sort_key".to_string(), "-start_date".to_string()),
    ];
    params.extend(collections.iter().map(|collection| {
        (
            "collection_concept_id".to_string(),
            (*collection).to_string(),
        )
    }));
    let start = asf_datetime(&query.start, false).ok().flatten();
    let end = asf_datetime(&query.end, true).ok().flatten();
    if start.is_some() || end.is_some() {
        params.push((
            "temporal".to_string(),
            format!("{},{}", start.unwrap_or_default(), end.unwrap_or_default()),
        ));
    }
    let Some(windows) = effective_bbox_windows(query) else {
        return if query.aoi_geojson.is_some() || query.bbox.is_some() {
            None
        } else {
            Some(vec![params])
        };
    };
    Some(
        windows
            .into_iter()
            .map(|bbox| {
                let mut variant = params.clone();
                variant.push((
                    "bounding_box".to_string(),
                    format!("{},{},{},{}", bbox.west, bbox.south, bbox.east, bbox.north),
                ));
                variant
            })
            .collect(),
    )
}

pub fn cmr_entry_to_feature(entry: &Map<String, Value>) -> Option<AsfRawFeature> {
    let mut source = text_value(
        entry
            .get("producer_granule_id")
            .or_else(|| entry.get("title"))
            .or_else(|| entry.get("id")),
    )?;
    if source.ends_with("-SLC") {
        source.truncate(source.len() - 4);
    }
    let parsed = parse_sentinel1_identifier(&source).ok()?;
    let attributes = attribute_map(entry);
    let mut properties = Map::from_iter([("sceneName".to_string(), Value::String(source))]);

    if let Some(url) = feature_url(entry) {
        properties.insert("url".to_string(), Value::String(url));
    }
    if let Some(megabytes) = entry.get("granule_size").and_then(numeric_value) {
        let bytes = (megabytes.trunc().max(0.0) as u64).saturating_mul(1024 * 1024);
        properties.insert("bytes".to_string(), Value::Number(Number::from(bytes)));
    }
    let remote_orbit = entry
        .get("orbit_calculated_spatial_domains")
        .and_then(Value::as_array)
        .and_then(|domains| domains.first())
        .and_then(Value::as_object)
        .and_then(|domain| domain.get("orbit_number"))
        .and_then(unsigned_value);
    if let Some(orbit) = remote_orbit {
        properties.insert("orbit".to_string(), Value::Number(Number::from(orbit)));
    }
    let path = attribute(
        &attributes,
        &["pathnumber", "path", "relativeorbit", "relativeorbitnumber"],
    )
    .and_then(unsigned_value)
    .or_else(|| {
        sentinel1_relative_orbit(
            &parsed.platform,
            remote_orbit.or(Some(parsed.absolute_orbit)),
        )
        .map(u64::from)
    });
    if let Some(path) = path {
        properties.insert("pathNumber".to_string(), Value::Number(Number::from(path)));
    }
    if let Some(frame) =
        attribute(&attributes, &["framenumber", "frame", "frameid"]).and_then(unsigned_value)
    {
        properties.insert(
            "frameNumber".to_string(),
            Value::Number(Number::from(frame)),
        );
    }
    if let Some(direction) = attribute(
        &attributes,
        &["ascendingdescending", "flightdirection", "orbitdirection"],
    )
    .and_then(|value| text_value(Some(value)))
    .and_then(normalize_direction)
    {
        properties.insert("flightDirection".to_string(), Value::String(direction));
    }
    let geometry = polygon_geometry(entry);
    Some(AsfRawFeature {
        r#type: "Feature".to_string(),
        properties,
        geometry,
    })
}

fn text_value(value: Option<&Value>) -> Option<String> {
    match value? {
        Value::String(text) if !text.trim().is_empty() => Some(text.trim().to_string()),
        Value::Number(number) => Some(number.to_string()),
        _ => None,
    }
}

fn numeric_value(value: &Value) -> Option<f64> {
    value
        .as_f64()
        .or_else(|| value.as_str().and_then(|text| text.trim().parse().ok()))
        .filter(|value| value.is_finite())
}

fn unsigned_value(value: &Value) -> Option<u64> {
    numeric_value(value)
        .and_then(|value| (value >= 0.0 && value.fract() == 0.0).then_some(value as u64))
}

fn feature_url(entry: &Map<String, Value>) -> Option<String> {
    entry
        .get("links")?
        .as_array()?
        .iter()
        .filter_map(Value::as_object)
        .find_map(|link| {
            let href = link.get("href")?.as_str()?.trim();
            if !href.starts_with("https://") {
                return None;
            }
            let relation = link.get("rel").and_then(Value::as_str).unwrap_or("");
            let title = link
                .get("title")
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_ascii_lowercase();
            (relation.contains("data#")
                || title.contains("direct download")
                || href.to_ascii_lowercase().ends_with(".zip"))
            .then(|| href.to_string())
        })
}

fn clean_key(value: &str) -> String {
    value
        .chars()
        .filter(|character| character.is_ascii_alphanumeric())
        .flat_map(char::to_lowercase)
        .collect()
}

fn attribute_map(entry: &Map<String, Value>) -> Map<String, Value> {
    let mut attributes = Map::new();
    for key in ["additional_attributes", "AdditionalAttributes"] {
        add_attribute_container(entry.get(key), &mut attributes);
    }
    if let Some(umm) = entry.get("umm").and_then(Value::as_object) {
        for key in ["AdditionalAttributes", "additional_attributes"] {
            add_attribute_container(umm.get(key), &mut attributes);
        }
    }
    attributes
}

fn add_attribute_container(value: Option<&Value>, target: &mut Map<String, Value>) {
    match value {
        Some(Value::Array(items)) => {
            for item in items.iter().filter_map(Value::as_object) {
                let name = item
                    .get("name")
                    .or_else(|| item.get("Name"))
                    .and_then(Value::as_str)
                    .map(clean_key);
                let value = item
                    .get("values")
                    .or_else(|| item.get("Values"))
                    .or_else(|| item.get("value"))
                    .or_else(|| item.get("Value"));
                if let (Some(name), Some(value)) = (name, first_non_empty(value)) {
                    target.entry(name).or_insert_with(|| value.clone());
                }
            }
        }
        Some(Value::Object(items)) => {
            for (name, value) in items {
                if let Some(value) = first_non_empty(Some(value)) {
                    target
                        .entry(clean_key(name))
                        .or_insert_with(|| value.clone());
                }
            }
        }
        _ => {}
    }
}

fn first_non_empty(value: Option<&Value>) -> Option<&Value> {
    match value? {
        Value::Array(items) => items.iter().find(|value| match value {
            Value::Null => false,
            Value::String(text) => !text.is_empty(),
            _ => true,
        }),
        Value::Null => None,
        Value::String(text) if text.is_empty() => None,
        value => Some(value),
    }
}

fn attribute<'a>(attributes: &'a Map<String, Value>, names: &[&str]) -> Option<&'a Value> {
    names.iter().find_map(|name| attributes.get(*name))
}

fn normalize_direction(value: String) -> Option<String> {
    let value = value.to_ascii_uppercase();
    if value.starts_with("ASC") {
        Some("ASCENDING".to_string())
    } else if value.starts_with("DESC") {
        Some("DESCENDING".to_string())
    } else {
        None
    }
}

fn polygon_geometry(entry: &Map<String, Value>) -> Option<Map<String, Value>> {
    let mut rings = Vec::new();
    for polygon in entry.get("polygons")?.as_array()? {
        let raw = match polygon {
            Value::Array(items) => items.first().and_then(Value::as_str),
            Value::String(text) => Some(text.as_str()),
            _ => None,
        };
        let Some(raw) = raw else {
            continue;
        };
        let numbers = raw
            .replace(',', " ")
            .split_whitespace()
            .filter_map(|text| text.parse::<f64>().ok())
            .collect::<Vec<_>>();
        let mut ring = numbers
            .chunks_exact(2)
            .filter_map(|pair| {
                let (latitude, longitude) = (pair[0], pair[1]);
                ((-90.0..=90.0).contains(&latitude) && (-180.0..=180.0).contains(&longitude))
                    .then(|| vec![Value::from(longitude), Value::from(latitude)])
            })
            .collect::<Vec<_>>();
        if ring.len() >= 3 {
            if ring.first() != ring.last() {
                ring.push(ring[0].clone());
            }
            rings.push(Value::Array(ring.into_iter().map(Value::Array).collect()));
        }
    }
    if rings.is_empty() {
        return None;
    }
    let (kind, coordinates) = if rings.len() == 1 {
        ("Polygon", Value::Array(rings))
    } else {
        (
            "MultiPolygon",
            Value::Array(
                rings
                    .into_iter()
                    .map(|ring| Value::Array(vec![ring]))
                    .collect(),
            ),
        )
    };
    Some(Map::from_iter([
        ("type".to_string(), Value::String(kind.to_string())),
        ("coordinates".to_string(), coordinates),
    ]))
}
