import { parseCanonicalGeometry } from "./geojsonGeometry.ts";
import { deriveWrappedAoiEnvelope, continuousDisplayBbox } from "./antimeridian.ts";

export type TaskFrozenSubsetAoi = {
  geometry: Record<string, unknown>;
  bbox: ReturnType<typeof continuousDisplayBbox>;
  frequency: "A" | "B";
  polarizations: string[];
};

function record(value: unknown): Record<string, unknown> | null {
  return value !== null && typeof value === "object" && !Array.isArray(value)
    ? value as Record<string, unknown> : null;
}

/** Display projection of already Store-validated intent; never a new execution authority. */
export function taskFrozenSubsetAoi(
  items: readonly { metadata: Record<string, unknown> }[],
): TaskFrozenSubsetAoi | null {
  if (items.length === 0) return null;
  let result: TaskFrozenSubsetAoi | null = null;
  let identity: string | null = null;
  for (const item of items) {
    if (item.metadata?.delivery_kind !== "remote_derived_subset_v2") return null;
    const envelope = record(item.metadata.nisar_subset_prepared_intent_v2_envelope);
    if (envelope?.envelope_contract !== "NisarSubsetPreparedIntentEnvelopeV2"
      || envelope.schema_version !== 2 || typeof envelope.canonical_intent_json !== "string"
      || envelope.canonical_intent_json.length > 2_000_000) return null;
    try {
      const intent = record(JSON.parse(envelope.canonical_intent_json));
      const geometry = parseCanonicalGeometry(record(intent?.requested_geometry)?.canonical_geojson, 1_000_000);
      const frequency = intent?.selected_frequency;
      const polarizations = intent?.selected_polarizations;
      if (!geometry || (frequency !== "A" && frequency !== "B")
        || !Array.isArray(polarizations) || polarizations.length === 0
        || !polarizations.every((value) => typeof value === "string" && /^(HH|HV|VH|VV)$/.test(value))) return null;
      const bounds = deriveWrappedAoiEnvelope(geometry);
      if (!bounds) return null;
      const candidate: TaskFrozenSubsetAoi = { geometry, bbox: continuousDisplayBbox(bounds), frequency, polarizations: [...polarizations] };
      const candidateIdentity = JSON.stringify([geometry, frequency, polarizations]);
      if (identity !== null && identity !== candidateIdentity) return null;
      identity = candidateIdentity;
      result = candidate;
    } catch { return null; }
  }
  return result;
}
