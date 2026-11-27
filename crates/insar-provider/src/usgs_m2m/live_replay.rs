use reqwest::Url;
use serde_json::{json, Map, Number, Value};

const MAX_REPLAY_RESULTS: usize = 101;
const MAX_REPLAY_METADATA_ITEMS: usize = 513;
const MAX_REPLAY_BROWSE_ITEMS: usize = 33;
const MAX_REPLAY_TEXT_BYTES: usize = 256;

const AUTHORITY_FIELDS: &[&str] = &[
    "Landsat Product Identifier L2",
    "Landsat Scene Identifier",
    "Date Acquired",
    "Collection Category",
    "WRS Path",
    "WRS Row",
    "Data Type L2",
    "Satellite",
];

const BROWSE_FIELDS: &[&str] = &[
    "id",
    "browseRotationEnabled",
    "browseName",
    "browsePath",
    "overlayPath",
    "overlayType",
    "thumbnailPath",
];

pub(super) fn redact_scene_search_data(value: &Value) -> Value {
    let Value::Object(input) = value else {
        return redact_type(value);
    };
    let mut output = Map::new();

    if let Some(results) = input.get("results") {
        output.insert(
            "results".to_owned(),
            match results {
                Value::Array(results) => Value::Array(
                    results
                        .iter()
                        .take(MAX_REPLAY_RESULTS)
                        .enumerate()
                        .map(|(index, result)| redact_scene(result, index))
                        .collect(),
                ),
                invalid => redact_type(invalid),
            },
        );
    }
    for field in [
        "recordsReturned",
        "totalHits",
        "startingNumber",
        "nextRecord",
        "numExcluded",
    ] {
        if let Some(value) = input.get(field) {
            output.insert(field.to_owned(), redact_public_u64(value));
        }
    }
    if let Some(value) = input.get("totalHitsAccuracy") {
        output.insert(
            "totalHitsAccuracy".to_owned(),
            match value.as_str() {
                Some("calculated") => Value::String("calculated".to_owned()),
                Some("approximate") => Value::String("approximate".to_owned()),
                Some(_) => Value::String("redacted".to_owned()),
                None => redact_type(value),
            },
        );
    }
    if let Some(value) = input.get("isCustomized") {
        output.insert(
            "isCustomized".to_owned(),
            value
                .as_bool()
                .map(Value::Bool)
                .unwrap_or_else(|| redact_type(value)),
        );
    }

    let recognized = [
        "results",
        "recordsReturned",
        "totalHits",
        "startingNumber",
        "nextRecord",
        "numExcluded",
        "totalHitsAccuracy",
        "isCustomized",
    ];
    insert_extra_count(&mut output, input, &recognized);
    output.insert(
        "_redaction".to_owned(),
        Value::String("usgs-m2m-scene-structure-v1".to_owned()),
    );
    Value::Object(output)
}

fn redact_scene(value: &Value, index: usize) -> Value {
    let Value::Object(input) = value else {
        return redact_type(value);
    };
    let mut output = Map::new();
    let synthetic_entity = format!("LC9{:019}", index + 1);
    let path = index % 233 + 1;
    let row = index % 248 + 1;
    let synthetic_product = format!("LC09_L2SP_{path:03}{row:03}_20000101_20000102_02_T1");

    if let Some(value) = input.get("entityId") {
        output.insert(
            "entityId".to_owned(),
            redact_string_as(value, &synthetic_entity),
        );
    }
    if let Some(value) = input.get("displayId") {
        output.insert(
            "displayId".to_owned(),
            redact_string_as(value, &synthetic_product),
        );
    }
    if let Some(value) = input.get("cloudCover") {
        output.insert("cloudCover".to_owned(), redact_number(value));
    }
    if let Some(value) = input.get("metadata") {
        output.insert(
            "metadata".to_owned(),
            redact_metadata(value, &synthetic_entity, &synthetic_product),
        );
    }
    if let Some(value) = input.get("browse") {
        output.insert("browse".to_owned(), redact_browse(value));
    }
    if let Some(value) = input.get("spatialCoverage") {
        output.insert("spatialCoverage".to_owned(), redact_geometry(value));
    }
    if let Some(value) = input.get("temporalCoverage") {
        output.insert("temporalCoverage".to_owned(), redact_temporal(value));
    }

    Value::Object(output)
}

fn redact_metadata(value: &Value, entity_id: &str, product_id: &str) -> Value {
    let Value::Array(items) = value else {
        return redact_type(value);
    };
    Value::Array(
        items
            .iter()
            .take(MAX_REPLAY_METADATA_ITEMS)
            .enumerate()
            .map(|(index, item)| redact_metadata_item(item, index, entity_id, product_id))
            .collect(),
    )
}

fn redact_metadata_item(value: &Value, index: usize, entity_id: &str, product_id: &str) -> Value {
    let Value::Object(input) = value else {
        return redact_type(value);
    };
    let mut output = Map::new();
    let authority_field = input
        .get("fieldName")
        .and_then(Value::as_str)
        .filter(|field| AUTHORITY_FIELDS.contains(field));

    if let Some(value) = input.get("fieldName") {
        output.insert(
            "fieldName".to_owned(),
            match value.as_str() {
                Some(field) if AUTHORITY_FIELDS.contains(&field) => Value::String(field.to_owned()),
                Some(_) => Value::String(format!("Other metadata field {index}")),
                None => redact_type(value),
            },
        );
    }
    if let Some(value) = input.get("value") {
        let redacted = match (authority_field, value) {
            (Some("Satellite"), Value::String(value)) => match value.as_str() {
                "LANDSAT_8" => Value::String("LANDSAT_8".to_owned()),
                "LANDSAT_9" => Value::String("LANDSAT_9".to_owned()),
                _ => Value::String("redacted".to_owned()),
            },
            (Some("Satellite"), Value::Number(value)) => {
                let satellite = match value.as_f64() {
                    Some(8.0) => 8_u64,
                    Some(9.0) => 9_u64,
                    _ => 0_u64,
                };
                Value::Number(Number::from(satellite))
            }
            (Some(field), Value::String(_)) => {
                Value::String(authority_value(field, entity_id, product_id).to_owned())
            }
            (None, Value::String(_)) => Value::String("redacted".to_owned()),
            (_, invalid) => redact_type(invalid),
        };
        output.insert("value".to_owned(), redacted);
        if let Value::String(value) = value {
            output.insert("_valueTextShape".to_owned(), redact_text_shape(value));
        }
    }
    Value::Object(output)
}

fn redact_text_shape(value: &str) -> Value {
    json!({
        "isEmpty": value.is_empty(),
        "hasOuterWhitespace": value != value.trim(),
        "hasControl": value.chars().any(char::is_control),
        "overMaximum": value.len() > MAX_REPLAY_TEXT_BYTES
    })
}

fn authority_value<'a>(field: &str, entity_id: &'a str, product_id: &'a str) -> &'a str {
    match field {
        "Landsat Product Identifier L2" => product_id,
        "Landsat Scene Identifier" => entity_id,
        "Date Acquired" => "2023/03/25",
        "Collection Category" => "T1",
        "WRS Path" | "WRS Row" => "001",
        "Data Type L2" => "OLI_TIRS_L2SP",
        "Satellite" => "LANDSAT_9",
        _ => "redacted",
    }
}

fn redact_browse(value: &Value) -> Value {
    let Value::Array(items) = value else {
        return redact_type(value);
    };
    Value::Array(
        items
            .iter()
            .take(MAX_REPLAY_BROWSE_ITEMS)
            .map(|item| {
                let Value::Object(input) = item else {
                    return redact_type(item);
                };
                let mut output = Map::new();
                for field in BROWSE_FIELDS {
                    if let Some(value) = input.get(*field) {
                        let redacted = match (*field, value) {
                            ("browseRotationEnabled", Value::Bool(value)) => Value::Bool(*value),
                            _ => redact_type(value),
                        };
                        output.insert((*field).to_owned(), redacted);
                        if matches!(*field, "browsePath" | "overlayPath" | "thumbnailPath") {
                            if let Some(structure) = browse_locator_structure(value) {
                                output.insert(format!("_{field}Structure"), structure);
                            }
                        }
                    }
                }
                insert_extra_count(&mut output, input, BROWSE_FIELDS);
                Value::Object(output)
            })
            .collect(),
    )
}

fn browse_locator_structure(value: &Value) -> Option<Value> {
    let value = value.as_str()?;
    if let Ok(url) = Url::parse(value) {
        let scheme = match url.scheme() {
            "https" => "https",
            "http" => "http",
            _ => "other",
        };
        let host = url
            .host_str()
            .filter(|host| valid_public_host(host))
            .unwrap_or("redacted");
        return Some(json!({
            "kind": "absolute_url",
            "scheme": scheme,
            "host": host,
            "effectivePort": url.port_or_known_default(),
            "hasUserinfo": !url.username().is_empty() || url.password().is_some(),
            "hasQuery": url.query().is_some(),
            "hasFragment": url.fragment().is_some()
        }));
    }
    Some(json!({
        "kind": if value.starts_with('/') { "absolute_path" } else { "relative_path" },
        "hasQuery": value.contains('?'),
        "hasFragment": value.contains('#')
    }))
}

fn valid_public_host(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 253
        && value.is_ascii()
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'-' | b':'))
}

fn redact_geometry(value: &Value) -> Value {
    let Value::Object(input) = value else {
        return redact_type(value);
    };
    let mut output = Map::new();
    let geometry_type = input.get("type").and_then(Value::as_str);
    if let Some(value) = input.get("type") {
        output.insert(
            "type".to_owned(),
            match geometry_type {
                Some("Polygon") => Value::String("Polygon".to_owned()),
                Some("MultiPolygon") => Value::String("MultiPolygon".to_owned()),
                Some(_) => Value::String("redacted".to_owned()),
                None => redact_type(value),
            },
        );
    }
    if let Some(value) = input.get("coordinates") {
        output.insert(
            "coordinates".to_owned(),
            match (geometry_type, value) {
                (Some("Polygon"), Value::Array(_)) => {
                    json!([[[0.0, 0.0], [1.0, 0.0], [1.0, 1.0], [0.0, 1.0], [0.0, 0.0]]])
                }
                (Some("MultiPolygon"), Value::Array(_)) => {
                    json!([[[[0.0, 0.0], [1.0, 0.0], [1.0, 1.0], [0.0, 1.0], [0.0, 0.0]]]])
                }
                _ => redact_type(value),
            },
        );
    }
    if let Some(value) = input.get("bbox") {
        output.insert("bbox".to_owned(), redact_type(value));
    }
    insert_extra_count(&mut output, input, &["type", "coordinates", "bbox"]);
    Value::Object(output)
}

fn redact_temporal(value: &Value) -> Value {
    let Value::Object(input) = value else {
        return redact_type(value);
    };
    let mut output = Map::new();
    for field in ["startDate", "endDate"] {
        if let Some(value) = input.get(field) {
            output.insert(
                field.to_owned(),
                redact_string_as(value, "2023-03-25 00:00:00"),
            );
        }
    }
    insert_extra_count(&mut output, input, &["startDate", "endDate"]);
    Value::Object(output)
}

fn redact_public_u64(value: &Value) -> Value {
    value
        .as_u64()
        .map(|value| Value::Number(Number::from(value)))
        .unwrap_or_else(|| redact_type(value))
}

fn redact_number(value: &Value) -> Value {
    if value.as_f64().is_some_and(f64::is_finite) {
        Value::Number(Number::from(0))
    } else {
        redact_type(value)
    }
}

fn redact_string_as(value: &Value, replacement: &str) -> Value {
    if value.is_string() {
        Value::String(replacement.to_owned())
    } else {
        redact_type(value)
    }
}

fn redact_type(value: &Value) -> Value {
    match value {
        Value::Null => Value::Null,
        Value::Bool(_) => Value::Bool(false),
        Value::Number(_) => Value::Number(Number::from(0)),
        Value::String(_) => Value::String("redacted".to_owned()),
        Value::Array(_) => Value::Array(Vec::new()),
        Value::Object(_) => Value::Object(Map::new()),
    }
}

fn insert_extra_count(output: &mut Map<String, Value>, input: &Map<String, Value>, known: &[&str]) {
    let count = input
        .keys()
        .filter(|key| !known.contains(&key.as_str()))
        .count();
    if count > 0 {
        output.insert(
            "_redacted_extra_field_count".to_owned(),
            Value::Number(Number::from(count as u64)),
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn redacted_replay_keeps_contract_shape_without_live_values() {
        let raw = json!({
            "results": [{
                "entityId": "secret-entity-id",
                "displayId": "secret-display-id",
                "cloudCover": 47.25,
                "metadata": [
                    {"fieldName": "Landsat Product Identifier L2", "value": "secret-display-id"},
                    {"fieldName": "account-secret-label", "value": "secret-application-token"},
                    {"fieldName": "Optional Null", "value": null},
                    {"fieldName": "Satellite", "value": 9},
                    {"fieldName": "Landsat Scene Identifier", "value": "  secret-scene-id  "}
                ],
                "browse": [{
                    "id": "secret-browse-id",
                    "browsePath": "https://example.invalid/image.jpg?token=secret",
                    "thumbnailPath": "https://example.invalid/thumb.jpg",
                    "unknownSecret": "secret-runtime-key"
                }],
                "spatialCoverage": {
                    "type": "Polygon",
                    "coordinates": [[[-75.446, 36.3793], [-76.9196, 35.5713], [-75.446, 36.3793]]]
                },
                "temporalCoverage": {
                    "startDate": "2023-03-26 00:00:00",
                    "endDate": "2023-03-26 00:00:00"
                },
                "remoteSecret": "secret-user"
            }],
            "recordsReturned": 1,
            "totalHits": 1,
            "startingNumber": 1,
            "nextRecord": 1,
            "totalHitsAccuracy": "calculated",
            "isCustomized": false,
            "numExcluded": 0,
            "unknownEnvelopeSecret": "secret-request-id"
        });

        let replay = redact_scene_search_data(&raw);
        let serialized = serde_json::to_string(&replay).unwrap();
        for forbidden in [
            "secret-",
            "account-secret-label",
            "https://",
            "-75.446",
            "36.3793",
            "unknownSecret",
            "remoteSecret",
            "unknownEnvelopeSecret",
        ] {
            assert!(!serialized.contains(forbidden));
        }
        assert_eq!(
            replay["results"][0]["metadata"][0]["fieldName"],
            "Landsat Product Identifier L2"
        );
        assert_eq!(replay["results"][0]["metadata"][2]["value"], Value::Null);
        assert_eq!(replay["results"][0]["metadata"][3]["value"], 9);
        assert_eq!(
            replay["results"][0]["metadata"][4]["_valueTextShape"],
            json!({
                "hasControl": false,
                "hasOuterWhitespace": true,
                "isEmpty": false,
                "overMaximum": false
            })
        );
        assert_eq!(replay["recordsReturned"], 1);
        assert_eq!(replay["results"][0]["browse"][0]["browsePath"], "redacted");
        assert_eq!(
            replay["results"][0]["browse"][0]["_browsePathStructure"]["host"],
            "example.invalid"
        );
        assert_eq!(
            replay["results"][0]["browse"][0]["_browsePathStructure"]["hasQuery"],
            true
        );
    }
}
