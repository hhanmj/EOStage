import assert from "node:assert/strict";
import test from "node:test";
import { taskFrozenSubsetAoi } from "./taskFrozenAoi.ts";

const geometry = { type: "Polygon", coordinates: [[[110, 30], [111, 30], [111, 31], [110, 30]]] };
function item(requested = geometry) {
  return { metadata: { delivery_kind: "remote_derived_subset_v2",
    nisar_subset_prepared_intent_v2_envelope: {
      envelope_contract: "NisarSubsetPreparedIntentEnvelopeV2", schema_version: 2,
      canonical_intent_json: JSON.stringify({ requested_geometry: { canonical_geojson: requested },
        selected_frequency: "B", selected_polarizations: ["HH", "HV"] }),
    },
  } };
}

test("historical AOI comes from Store-frozen intent and returns an isolated geometry", () => {
  const stored = item();
  const before = structuredClone(stored);
  const result = taskFrozenSubsetAoi([stored, item()]);
  assert.ok(result);
  assert.deepEqual(result.geometry, geometry);
  assert.equal(result.frequency, "B");
  assert.deepEqual(result.polarizations, ["HH", "HV"]);
  assert.equal(result.bbox.west, 110);
  (result.geometry.coordinates as number[][][])[0]![0]![0] = 50;
  assert.deepEqual(stored, before);
  assert.deepEqual(taskFrozenSubsetAoi([stored])?.geometry, geometry);
});

test("missing, malformed, or conflicting frozen intent never manufactures a replacement AOI", () => {
  assert.equal(taskFrozenSubsetAoi([]), null);
  assert.equal(taskFrozenSubsetAoi([{ metadata: {} }]), null);
  const malformed = item();
  malformed.metadata.nisar_subset_prepared_intent_v2_envelope.canonical_intent_json = "broken";
  assert.equal(taskFrozenSubsetAoi([malformed]), null);
  assert.equal(taskFrozenSubsetAoi([item(), item({ ...geometry, coordinates: [[[120, 20], [121, 20], [121, 21], [120, 20]]] })]), null);
  assert.equal(taskFrozenSubsetAoi([item({ type: "Polygon", coordinates: [] })]), null);
});
