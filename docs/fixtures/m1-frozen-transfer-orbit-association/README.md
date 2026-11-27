# M1 frozen-transfer / Orbit-association proposed fixtures

**Status:** proposed design vectors only. They are not current canonical-schema
fixtures, production tests, live Provider evidence, or proof that a V3 type has
been implemented.

## Purpose and boundary

These vectors review the Gate B.1 proposal without changing
contracts/v3/provider-contract.schema.json. The proposed primary payload is the
independent, closed FrozenTaskSnapshotV3; the two explicitly labelled
compatibility vectors carry the current closed FrozenTaskSnapshot (legacy V2)
instead. They never claim that the current 3.12.0 schema accepts a V3 payload.

Every vector has exactly this harness boundary:

~~~json
{
  "fixture_metadata": {
    "fixture_version": 1,
    "fixture_kind": "...",
    "schema_contract_candidate": "3.13.0",
    "protocol_minor": 17,
    "payload_contract": "FrozenTaskSnapshotV3",
    "status": "proposed_design_vector"
  },
  "expected": {},
  "payload": {}
}
~~~

payload is the complete object named by payload_contract, not a nested task,
partial fragment, normalized view, or test harness. There are no
inherited_parent_facts. A V3 payload includes all parent task fields:
persistence_version, schema_version, task_id, batch_id, provider_id, created_at,
aoi_name, aoi_geojson, query, source_items, initial_selected_item_ids,
output_root, layout, runtime_item_identity, transfer_items, and
orbit_associations.

The legacy compatibility payloads are complete current V2 FrozenTaskSnapshot
objects. They preserve candidates and do not invent persistence_version,
transfer IDs, associations, or frozen access for an item whose V2 asset_access
is null. Their expected result may describe an in-memory LegacyTransferItemView,
but no such view appears in payload.

## Data and privacy rules

All IDs, timestamps, AOI coordinates, product keys, file names, and paths are
deterministic fixtures. Source metadata and payload are empty objects: platform
and source interval are deliberately represented only by typed association
fields. The transfer object has no outer role; its only role fact is
asset_access.role.

temporary-url-rejected.json contains the only URL in this directory: a
deliberately nonfunctional example.invalid URL with the literal marker
REDACTED_FIXTURE_VALUE. It is a controlled negative vector for the existing
temporary-URL prohibition, not a credential or a usable locator.

## Vector matrix

| Vector | Class | Expected result |
| --- | --- | --- |
| legacy-v2-task.json | compatibility | Read V2 without rewrite or fabricated access. |
| v2-to-v3-normalized-view.json | compatibility | Persisted V2 stays V2; only an in-memory legacy view may be projected. |
| single-source-single-poeorb-valid.json | valid | One source, primary transfer, and exact required POEORB transfer. |
| multiple-sources-shared-poeorb-valid.json | valid | Two sources share one POEORB transfer and its one recovery history. |
| missing-orbit.json | selection rejection | ORB001. |
| ambiguous-orbit.json | selection rejection | ORB001. |
| platform-mismatch.json | selection rejection | ORB001. |
| validity-gap.json | selection rejection | ORB001. |
| duplicate-source-association.json | structural rejection | V3_STORE_INVALID_SNAPSHOT. |
| orphan-transfer-item.json | structural rejection | V3_STORE_INVALID_SNAPSHOT. |
| temporary-url-rejected.json | structural rejection | V3_STORE_INVALID_SNAPSHOT. |
| protocol-16-association-rejected.json | protocol rejection | V3_SIDECAR_CAPABILITY_UNAVAILABLE. |
| duplicate-transfer-item-id.json | structural rejection | V3_STORE_INVALID_SNAPSHOT. |
| duplicate-source-item-id.json | structural rejection | V3_STORE_INVALID_SNAPSHOT. |
| selected-transfer-reference-mismatch.json | structural rejection | V3_STORE_INVALID_SNAPSHOT. |
| orbit-filename-mismatch.json | structural rejection | V3_STORE_INVALID_SNAPSHOT. |
| attachment-association-set-mismatch.json | structural rejection | V3_STORE_INVALID_SNAPSHOT. |
| association-source-not-selected.json | structural rejection | V3_STORE_INVALID_SNAPSHOT. |
| source-interval-invalid.json | structural rejection | V3_STORE_INVALID_SNAPSHOT. |
| companion-not-required.json | structural rejection | V3_STORE_INVALID_SNAPSHOT. |

fixture-index.json is the authoritative fixture list. A future canonical test
may consume these vectors only after the separately approved B1 schema and
validator work exists.
