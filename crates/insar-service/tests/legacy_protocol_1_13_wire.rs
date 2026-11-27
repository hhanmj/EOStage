use serde::Deserialize;
use serde_json::{Map, Value};

#[allow(dead_code)]
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct FrozenSidecarRequestV113 {
    protocol_major: u64,
    protocol_minor: u64,
    request_id: String,
    operation: String,
    payload: FrozenStoreSearchBatchPayloadV113,
}

#[allow(dead_code)]
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct FrozenStoreSearchBatchPayloadV113 {
    batch: FrozenSearchBatchV113,
    workbench: FrozenSearchWorkbenchStateV113,
    activate_workbench: bool,
}

#[allow(dead_code)]
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct FrozenSearchBatchV113 {
    schema_version: u64,
    batch_id: String,
    provider_id: String,
    created_at: String,
    aoi_name: String,
    aoi_geojson: Value,
    query: Map<String, Value>,
    candidates: Vec<FrozenSnapshotItemV113>,
}

#[allow(dead_code)]
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct FrozenSnapshotItemV113 {
    item_id: String,
    provider_id: String,
    display_name: String,
    acquisition_datetime: Option<String>,
    footprint_geojson: Option<Value>,
    metadata: Map<String, Value>,
    payload: Map<String, Value>,
    asset_access: Option<Value>,
}

#[allow(dead_code)]
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct FrozenSearchWorkbenchStateV113 {
    batch_id: String,
    checked_item_ids: Vec<String>,
    page_index: u64,
    page_size: u64,
    revision: u64,
}

fn compatibility_fixture() -> &'static str {
    include_str!("../../../tests/fixtures/v3_contracts/protocol_1_13_store_search_batch.json")
}

#[test]
fn frozen_protocol_1_13_decoder_accepts_compatibility_wire() {
    let request: FrozenSidecarRequestV113 =
        serde_json::from_str(compatibility_fixture()).expect("frozen 1.13 wire decodes");

    assert_eq!(request.protocol_minor, 13);
    assert_eq!(request.operation, "store_search_batch");
    let candidate = &request.payload.batch.candidates[0];
    assert!(candidate.acquisition_datetime.is_none());
    assert!(candidate.footprint_geojson.is_none());
    assert!(candidate.asset_access.is_none());
}

#[test]
fn frozen_protocol_1_13_decoder_rejects_even_null_execution_policy() {
    let mut wire: Value =
        serde_json::from_str(compatibility_fixture()).expect("fixture JSON is valid");
    wire.pointer_mut("/payload/batch/candidates/0")
        .and_then(Value::as_object_mut)
        .expect("fixture candidate is an object")
        .insert("execution_policy".to_string(), Value::Null);

    let decoded = serde_json::from_value::<FrozenSidecarRequestV113>(wire);
    assert!(decoded.is_err());
}
