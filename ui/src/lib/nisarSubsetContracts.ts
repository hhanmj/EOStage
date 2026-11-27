export const NISAR_REMOTE_SUBSET_METADATA_KEY = "nisar_remote_subset" as const;
export const NISAR_SUBSET_REQUEST_MANIFEST_CONTRACT = "NisarSubsetRequestManifestV1" as const;
export const NISAR_SUBSET_TRANSFER_MANIFEST_CONTRACT = "NisarSubsetTransferManifestV1" as const;
export const NISAR_SUBSET_ARTIFACT_MANIFEST_CONTRACT = "NisarSubsetArtifactManifestV1" as const;
export const NISAR_SUBSET_SCHEMA_VERSION = 1 as const;
export const NISAR_SUBSET_MECHANISM = "RemoteHdf5SpatialWindow" as const;
export const NISAR_SUBSET_IMPLEMENTATION_SCOPE = "offline_only" as const;
export const NISAR_SUBSET_OUTPUT_AUTHORITY = "client_derived" as const;
export const NISAR_SUBSET_METHOD = "range_hyperslab" as const;
export const NISAR_SUBSET_MAPPING_ALGORITHM = "sparse_grid_polygon_cells_v1" as const;
export const NISAR_SUBSET_MAPPING_ALGORITHM_VERSION = 1 as const;
export const NISAR_SUBSET_VALIDITY_MASK_SUFFIX = "inputDataExceptionMask" as const;
export const NISAR_SUBSET_MAX_CANONICAL_JSON_BYTES = 32 * 1024;
export const NISAR_SUBSET_MAX_TRANSFER_CANONICAL_JSON_BYTES = 2 * 1024 * 1024;
export const NISAR_SUBSET_MAX_ARTIFACT_CANONICAL_JSON_BYTES = 1024 * 1024;
export const NISAR_SUBSET_MAX_WINDOWS = 64;
export const NISAR_SUBSET_MAX_COMPONENTS = 64;
export const NISAR_SUBSET_MAX_RINGS = 256;
export const NISAR_SUBSET_MAX_VERTICES = 4096;
export const NISAR_SUBSET_MAX_RANGES = 4096;
export const NISAR_SUBSET_MAX_STRING_BYTES = 512;
export const NISAR_SUBSET_MAX_SAFE_INTEGER = Number.MAX_SAFE_INTEGER;

export type Frequency = "A" | "B";
export type MappingConfidence = "conservative" | "reference_bounded";
export type ChecksumAlgorithm = "md5" | "sha1" | "sha256";
export type ValidatorKind = "strong_etag" | "last_modified";
export type RangeState = "PLANNED" | "IN_PROGRESS" | "COMPLETE" | "FAILED";
export type TransferState =
  | "PLANNED"
  | "IN_PROGRESS"
  | "INTERRUPTED"
  | "SOURCE_CHANGED"
  | "RANGES_COMPLETE"
  | "FAILED";
export type CheckStatus = "passed" | "failed" | "not_run";

export class ManifestContractError extends Error {
  readonly code: string;

  constructor(code: string, detail: string) {
    super(`${code}: ${detail}`);
    this.name = "ManifestContractError";
    this.code = code;
  }
}

export interface NisarSubsetChecksumV1 {
  algorithm: ChecksumAlgorithm;
  value: string;
}

export interface NisarSubsetSourceV1 {
  provider_id: string;
  collection_id: string;
  collection_profile_id: string;
  provider_product_id: string;
  provider_product_key: string;
  stable_asset_key: string;
  file_name: string;
  expected_size_bytes: number;
  checksum?: NisarSubsetChecksumV1;
  product_specification_version: string;
  crid: string;
}

export interface NisarSubsetRequestV1 {
  aoi_sha256: string;
  aoi_crs: string;
  aoi_component_count: number;
  aoi_ring_count: number;
  aoi_vertex_count: number;
  frequency: Frequency;
  polarization: string;
  swath: string;
  subset_method: string;
}

export interface NisarSubsetLogicalWindowV1 {
  component_id: number;
  dataset_path: string;
  frequency: Frequency;
  polarization: string;
  swath: string;
  line_start: number;
  line_end_exclusive: number;
  sample_start: number;
  sample_end_exclusive: number;
  source_shape: [number, number];
  expected_shape: [number, number];
  dtype: string;
  chunk_shape: [number, number];
  margin_lines: number;
  margin_samples: number;
  mapping_confidence: MappingConfidence;
  validity_mask_required: boolean;
}

export interface NisarSubsetMappingV1 {
  algorithm: string;
  algorithm_version: number;
  evidence_digest: string;
  windows: NisarSubsetLogicalWindowV1[];
}

export interface NisarSubsetExpectedOutputV1 {
  derived_artifact_id: string;
  file_name: string;
  estimated_output_bytes: number;
  estimated_remote_bytes: number;
  full_product_bytes: number;
}

export interface NisarSubsetRequestManifestV1 {
  manifest_contract: typeof NISAR_SUBSET_REQUEST_MANIFEST_CONTRACT;
  schema_version: typeof NISAR_SUBSET_SCHEMA_VERSION;
  mechanism: typeof NISAR_SUBSET_MECHANISM;
  implementation_scope: typeof NISAR_SUBSET_IMPLEMENTATION_SCOPE;
  output_authority: typeof NISAR_SUBSET_OUTPUT_AUTHORITY;
  default_enabled: false;
  source: NisarSubsetSourceV1;
  request: NisarSubsetRequestV1;
  mapping: NisarSubsetMappingV1;
  expected_output: NisarSubsetExpectedOutputV1;
}

export interface NisarSubsetSourceValidatorV1 {
  kind: ValidatorKind;
  value: string;
  content_length: number;
}

export interface NisarSubsetByteRangeV1 {
  start: number;
  end_exclusive: number;
  expected_bytes: number;
  received_bytes: number;
  sha256?: string;
  state: RangeState;
}

export interface NisarSubsetTransferManifestV1 {
  manifest_contract: typeof NISAR_SUBSET_TRANSFER_MANIFEST_CONTRACT;
  schema_version: typeof NISAR_SUBSET_SCHEMA_VERSION;
  request_manifest_sha256: string;
  source_identity: NisarSubsetSourceV1;
  source_validator: NisarSubsetSourceValidatorV1;
  ranges: NisarSubsetByteRangeV1[];
  expected_remote_bytes: number;
  received_bytes: number;
  state: TransferState;
  created_at: string;
  updated_at: string;
}

export interface NisarSubsetValidationCheckV1 {
  status: CheckStatus;
  evidence: string;
}

export interface NisarSubsetActualExtentV1 {
  line_start: number;
  line_end_exclusive: number;
  sample_start: number;
  sample_end_exclusive: number;
  component_count: number;
}

export interface NisarSubsetValidityMaskV1 {
  component_id: number;
  dataset_path: string;
  frequency: Frequency;
  swath: string;
  line_start: number;
  line_end_exclusive: number;
  sample_start: number;
  sample_end_exclusive: number;
  shape: [number, number];
  sha256: string;
  semantics: string;
}

export interface NisarSubsetWriterV1 {
  writer_id: string;
  version: number;
}

export interface NisarSubsetValidatorPolicyV1 {
  policy_id: string;
  version: number;
  product_specification_version: string;
}

export interface NisarSubsetScienceValidationV1 {
  structure: NisarSubsetValidationCheckV1;
  complex_values: NisarSubsetValidationCheckV1;
  phase: NisarSubsetValidationCheckV1;
  amplitude: NisarSubsetValidationCheckV1;
  coordinates: NisarSubsetValidationCheckV1;
  metadata: NisarSubsetValidationCheckV1;
  provenance: NisarSubsetValidationCheckV1;
  recovery: NisarSubsetValidationCheckV1;
}

export interface NisarSubsetArtifactManifestV1 {
  manifest_contract: typeof NISAR_SUBSET_ARTIFACT_MANIFEST_CONTRACT;
  schema_version: typeof NISAR_SUBSET_SCHEMA_VERSION;
  request_manifest_sha256: string;
  transfer_manifest_sha256: string;
  output_authority: typeof NISAR_SUBSET_OUTPUT_AUTHORITY;
  artifact_id: string;
  artifact_file_name: string;
  artifact_size_bytes: number;
  artifact_sha256: string;
  source_product_identity: NisarSubsetSourceV1;
  actual_extent: NisarSubsetActualExtentV1;
  dataset_windows: NisarSubsetLogicalWindowV1[];
  validity_masks: NisarSubsetValidityMaskV1[];
  writer: NisarSubsetWriterV1;
  validator: NisarSubsetValidatorPolicyV1;
  science_validation: NisarSubsetScienceValidationV1;
  remote_bytes: number;
  full_product_bytes: number;
  complete: true;
}

type ObjectValue = Record<string, unknown>;

export function parseRequestManifest(value: unknown): NisarSubsetRequestManifestV1 {
  const object = exactObject(value, [
    "manifest_contract",
    "schema_version",
    "mechanism",
    "implementation_scope",
    "output_authority",
    "default_enabled",
    "source",
    "request",
    "mapping",
    "expected_output",
  ]);
  const manifest = {
    manifest_contract: literal(object, "manifest_contract", NISAR_SUBSET_REQUEST_MANIFEST_CONTRACT),
    schema_version: literal(object, "schema_version", NISAR_SUBSET_SCHEMA_VERSION),
    mechanism: literal(object, "mechanism", NISAR_SUBSET_MECHANISM),
    implementation_scope: literal(object, "implementation_scope", NISAR_SUBSET_IMPLEMENTATION_SCOPE),
    output_authority: literal(object, "output_authority", NISAR_SUBSET_OUTPUT_AUTHORITY),
    default_enabled: literal(object, "default_enabled", false),
    source: parseSource(object.source),
    request: parseRequest(object.request),
    mapping: parseMapping(object.mapping),
    expected_output: parseExpectedOutput(object.expected_output),
  } satisfies NisarSubsetRequestManifestV1;
  validateRequestManifest(manifest);
  canonicalJsonValidated(manifest);
  return manifest;
}

export function parseTransferManifest(value: unknown): NisarSubsetTransferManifestV1 {
  const object = exactObject(value, [
    "manifest_contract",
    "schema_version",
    "request_manifest_sha256",
    "source_identity",
    "source_validator",
    "ranges",
    "expected_remote_bytes",
    "received_bytes",
    "state",
    "created_at",
    "updated_at",
  ]);
  const manifest: NisarSubsetTransferManifestV1 = {
    manifest_contract: literal(object, "manifest_contract", NISAR_SUBSET_TRANSFER_MANIFEST_CONTRACT),
    schema_version: literal(object, "schema_version", NISAR_SUBSET_SCHEMA_VERSION),
    request_manifest_sha256: hash(object.request_manifest_sha256, "request_manifest_sha256"),
    source_identity: parseSource(object.source_identity),
    source_validator: parseSourceValidator(object.source_validator),
    ranges: array(object.ranges, "ranges", parseRange),
    expected_remote_bytes: positiveInteger(object.expected_remote_bytes, "expected_remote_bytes"),
    received_bytes: nonNegativeInteger(object.received_bytes, "received_bytes"),
    state: enumValue(object.state, ["PLANNED", "IN_PROGRESS", "INTERRUPTED", "SOURCE_CHANGED", "RANGES_COMPLETE", "FAILED"] as const, "state"),
    created_at: safeText(object.created_at, "created_at", 64),
    updated_at: safeText(object.updated_at, "updated_at", 64),
  };
  validateTransferManifest(manifest);
  canonicalJsonValidated(manifest);
  return manifest;
}

export function parseArtifactManifest(value: unknown): NisarSubsetArtifactManifestV1 {
  const object = exactObject(value, [
    "manifest_contract",
    "schema_version",
    "request_manifest_sha256",
    "transfer_manifest_sha256",
    "output_authority",
    "artifact_id",
    "artifact_file_name",
    "artifact_size_bytes",
    "artifact_sha256",
    "source_product_identity",
    "actual_extent",
    "dataset_windows",
    "validity_masks",
    "writer",
    "validator",
    "science_validation",
    "remote_bytes",
    "full_product_bytes",
    "complete",
  ]);
  const manifest: NisarSubsetArtifactManifestV1 = {
    manifest_contract: literal(object, "manifest_contract", NISAR_SUBSET_ARTIFACT_MANIFEST_CONTRACT),
    schema_version: literal(object, "schema_version", NISAR_SUBSET_SCHEMA_VERSION),
    request_manifest_sha256: hash(object.request_manifest_sha256, "request_manifest_sha256"),
    transfer_manifest_sha256: hash(object.transfer_manifest_sha256, "transfer_manifest_sha256"),
    output_authority: literal(object, "output_authority", NISAR_SUBSET_OUTPUT_AUTHORITY),
    artifact_id: safeText(object.artifact_id, "artifact_id", NISAR_SUBSET_MAX_STRING_BYTES),
    artifact_file_name: derivedFileName(object.artifact_file_name, "artifact_file_name"),
    artifact_size_bytes: positiveInteger(object.artifact_size_bytes, "artifact_size_bytes"),
    artifact_sha256: hash(object.artifact_sha256, "artifact_sha256"),
    source_product_identity: parseSource(object.source_product_identity),
    actual_extent: parseExtent(object.actual_extent),
    dataset_windows: array(object.dataset_windows, "dataset_windows", parseWindow),
    validity_masks: array(object.validity_masks, "validity_masks", parseMask),
    writer: parseWriter(object.writer),
    validator: parseValidator(object.validator),
    science_validation: parseScienceValidation(object.science_validation),
    remote_bytes: positiveInteger(object.remote_bytes, "remote_bytes"),
    full_product_bytes: positiveInteger(object.full_product_bytes, "full_product_bytes"),
    complete: literal(object, "complete", true),
  };
  validateArtifactManifest(manifest);
  canonicalJsonValidated(manifest);
  return manifest;
}

export function parseRequestManifestJson(value: string): NisarSubsetRequestManifestV1 {
  return parseRequestManifest(parseJson(value, NISAR_SUBSET_MAX_CANONICAL_JSON_BYTES));
}

export function parseTransferManifestJson(value: string): NisarSubsetTransferManifestV1 {
  return parseTransferManifest(parseJson(value, NISAR_SUBSET_MAX_TRANSFER_CANONICAL_JSON_BYTES));
}

export function parseArtifactManifestJson(value: string): NisarSubsetArtifactManifestV1 {
  return parseArtifactManifest(parseJson(value, NISAR_SUBSET_MAX_ARTIFACT_CANONICAL_JSON_BYTES));
}

export function embedRequestManifest(
  metadata: Record<string, unknown>,
  manifest: NisarSubsetRequestManifestV1,
): void {
  const parsed = parseRequestManifest(manifest);
  metadata[NISAR_REMOTE_SUBSET_METADATA_KEY] = cloneJson(parsed);
}

export function parseRequestManifestFromMetadata(
  metadata: Record<string, unknown>,
): NisarSubsetRequestManifestV1 {
  if (!(NISAR_REMOTE_SUBSET_METADATA_KEY in metadata)) {
    throw new ManifestContractError("NISAR_SUBSET_MANIFEST_MISSING", NISAR_REMOTE_SUBSET_METADATA_KEY);
  }
  return parseRequestManifest(metadata[NISAR_REMOTE_SUBSET_METADATA_KEY]);
}

export function validateRequestAgainstSnapshotItem(
  requestValue: unknown,
  itemValue: unknown,
): void {
  const request = parseRequestManifest(requestValue);
  const item = record(itemValue, "snapshot item");
  const asset = record(item.asset_access, "snapshot asset_access");
  const reference = record(asset.reference, "snapshot asset reference");
  const execution = record(item.execution_policy, "snapshot execution policy");
  const source = request.source;
  if (item.provider_id !== source.provider_id
    || reference.provider_id !== source.provider_id
    || reference.collection_id !== source.collection_id
    || reference.provider_product_id !== source.provider_product_id
    || reference.provider_product_key !== source.provider_product_key
    || reference.stable_asset_key !== source.stable_asset_key
    || execution.collection_profile_id !== source.collection_profile_id) {
    throw new ManifestContractError("NISAR_SUBSET_IDENTITY_MISMATCH", "snapshot product identity");
  }
  if (asset.role !== "primary"
    || asset.file_name !== source.file_name
    || !source.file_name.toLowerCase().endsWith(".h5")
    || asset.expected_size_bytes !== source.expected_size_bytes) {
    throw new ManifestContractError("NISAR_SUBSET_IDENTITY_MISMATCH", "snapshot primary HDF5");
  }
  const checksumMatches = source.checksum === undefined
    ? asset.checksum_algorithm === undefined && asset.checksum_value === undefined
    : typeof asset.checksum_algorithm === "string"
      && asset.checksum_algorithm.toLowerCase() === source.checksum.algorithm
      && asset.checksum_value === source.checksum.value;
  if (!checksumMatches) {
    throw new ManifestContractError("NISAR_SUBSET_IDENTITY_MISMATCH", "snapshot checksum");
  }
}

function record(value: unknown, label: string): ObjectValue {
  if (value === null || typeof value !== "object" || Array.isArray(value)) {
    throw new ManifestContractError("NISAR_SUBSET_IDENTITY_MISMATCH", label);
  }
  return value as ObjectValue;
}

function parseManifest(value: unknown): NisarSubsetManifestV1 {
  if (value === null || typeof value !== "object" || Array.isArray(value)) {
    throw new ManifestContractError("NISAR_SUBSET_MANIFEST_INVALID", "manifest object expected");
  }
  const contract = (value as ObjectValue).manifest_contract;
  if (contract === NISAR_SUBSET_REQUEST_MANIFEST_CONTRACT) return parseRequestManifest(value);
  if (contract === NISAR_SUBSET_TRANSFER_MANIFEST_CONTRACT) return parseTransferManifest(value);
  if (contract === NISAR_SUBSET_ARTIFACT_MANIFEST_CONTRACT) return parseArtifactManifest(value);
  throw new ManifestContractError("NISAR_SUBSET_MANIFEST_INVALID", "unknown manifest contract");
}

export type NisarSubsetManifestV1 =
  | NisarSubsetRequestManifestV1
  | NisarSubsetTransferManifestV1
  | NisarSubsetArtifactManifestV1;

export function canonicalJson(value: unknown): string {
  return canonicalJsonValidated(parseManifest(value));
}

function canonicalJsonValidated(manifest: NisarSubsetManifestV1): string {
  let value = cloneJson(manifest) as ObjectValue;
  if (value.manifest_contract === NISAR_SUBSET_REQUEST_MANIFEST_CONTRACT) {
    const mapping = value.mapping as ObjectValue;
    mapping.windows = [...(mapping.windows as unknown[])].sort(compareWindowValues);
  } else if (value.manifest_contract === NISAR_SUBSET_TRANSFER_MANIFEST_CONTRACT) {
    value.ranges = [...(value.ranges as unknown[])].sort(compareRangeValues);
  } else if (value.manifest_contract === NISAR_SUBSET_ARTIFACT_MANIFEST_CONTRACT) {
    value.dataset_windows = [...(value.dataset_windows as unknown[])].sort(compareWindowValues);
    value.validity_masks = [...(value.validity_masks as unknown[])].sort(compareMaskValues);
  }
  const rendered = JSON.stringify(sortJson(value));
  const maximum = manifest.manifest_contract === NISAR_SUBSET_TRANSFER_MANIFEST_CONTRACT
    ? NISAR_SUBSET_MAX_TRANSFER_CANONICAL_JSON_BYTES
    : manifest.manifest_contract === NISAR_SUBSET_ARTIFACT_MANIFEST_CONTRACT
      ? NISAR_SUBSET_MAX_ARTIFACT_CANONICAL_JSON_BYTES
      : NISAR_SUBSET_MAX_CANONICAL_JSON_BYTES;
  if (new TextEncoder().encode(rendered).length > maximum) {
    throw new ManifestContractError("NISAR_SUBSET_MANIFEST_TOO_LARGE", String(maximum));
  }
  return rendered;
}

export async function canonicalSha256(
  manifest: unknown,
): Promise<string> {
  const bytes = new TextEncoder().encode(canonicalJson(manifest));
  const digest = await globalThis.crypto.subtle.digest("SHA-256", bytes);
  return [...new Uint8Array(digest)].map((byte) => byte.toString(16).padStart(2, "0")).join("");
}

function parseSource(value: unknown): NisarSubsetSourceV1 {
  const object = exactObject(value, [
    "provider_id",
    "collection_id",
    "collection_profile_id",
    "provider_product_id",
    "provider_product_key",
    "stable_asset_key",
    "file_name",
    "expected_size_bytes",
    "product_specification_version",
    "crid",
  ], ["checksum"]);
  const fileName = safeText(object.file_name, "source.file_name", NISAR_SUBSET_MAX_STRING_BYTES);
  if (fileName.includes("/") || fileName.includes("\\") || fileName.endsWith(".part")) {
    throw new ManifestContractError("NISAR_SUBSET_IDENTITY_MISMATCH", "source.file_name");
  }
  return {
    provider_id: safeText(object.provider_id, "provider_id", NISAR_SUBSET_MAX_STRING_BYTES),
    collection_id: safeText(object.collection_id, "collection_id", NISAR_SUBSET_MAX_STRING_BYTES),
    collection_profile_id: safeText(object.collection_profile_id, "collection_profile_id", NISAR_SUBSET_MAX_STRING_BYTES),
    provider_product_id: safeText(object.provider_product_id, "provider_product_id", NISAR_SUBSET_MAX_STRING_BYTES),
    provider_product_key: safeText(object.provider_product_key, "provider_product_key", NISAR_SUBSET_MAX_STRING_BYTES),
    stable_asset_key: safeText(object.stable_asset_key, "stable_asset_key", NISAR_SUBSET_MAX_STRING_BYTES),
    file_name: fileName,
    expected_size_bytes: positiveInteger(object.expected_size_bytes, "source.expected_size_bytes"),
    ...(object.checksum === undefined ? {} : {checksum: parseChecksum(object.checksum)}),
    product_specification_version: safeText(object.product_specification_version, "product_specification_version", NISAR_SUBSET_MAX_STRING_BYTES),
    crid: safeText(object.crid, "crid", NISAR_SUBSET_MAX_STRING_BYTES),
  };
}

function parseChecksum(value: unknown): NisarSubsetChecksumV1 {
  const object = exactObject(value, ["algorithm", "value"]);
  const algorithm = enumValue(object.algorithm, ["md5", "sha1", "sha256"] as const, "checksum.algorithm");
  const digest = stringValue(object.value, "checksum.value");
  const length = algorithm === "md5" ? 32 : algorithm === "sha1" ? 40 : 64;
  if (digest.length !== length || !/^[0-9a-f]+$/.test(digest)) {
    throw new ManifestContractError("NISAR_SUBSET_IDENTITY_MISMATCH", "checksum.value");
  }
  return {algorithm, value: digest};
}

function parseRequest(value: unknown): NisarSubsetRequestV1 {
  const object = exactObject(value, [
    "aoi_sha256",
    "aoi_crs",
    "aoi_component_count",
    "aoi_ring_count",
    "aoi_vertex_count",
    "frequency",
    "polarization",
    "swath",
    "subset_method",
  ]);
  return {
    aoi_sha256: hash(object.aoi_sha256, "aoi_sha256"),
    aoi_crs: literal(object, "aoi_crs", "EPSG:4326"),
    aoi_component_count: boundedInteger(object.aoi_component_count, "aoi_component_count", 1, NISAR_SUBSET_MAX_COMPONENTS),
    aoi_ring_count: boundedInteger(object.aoi_ring_count, "aoi_ring_count", 1, NISAR_SUBSET_MAX_RINGS),
    aoi_vertex_count: boundedInteger(object.aoi_vertex_count, "aoi_vertex_count", 1, NISAR_SUBSET_MAX_VERTICES),
    frequency: enumValue(object.frequency, ["A", "B"] as const, "frequency"),
    polarization: stringValue(object.polarization, "polarization"),
    swath: safeText(object.swath, "swath", 128),
    subset_method: literal(object, "subset_method", NISAR_SUBSET_METHOD),
  };
}

function parseMapping(value: unknown): NisarSubsetMappingV1 {
  const object = exactObject(value, ["algorithm", "algorithm_version", "evidence_digest", "windows"]);
  return {
    algorithm: literal(object, "algorithm", NISAR_SUBSET_MAPPING_ALGORITHM),
    algorithm_version: literal(object, "algorithm_version", NISAR_SUBSET_MAPPING_ALGORITHM_VERSION),
    evidence_digest: hash(object.evidence_digest, "mapping.evidence_digest"),
    windows: array(object.windows, "mapping.windows", parseWindow),
  };
}

function parseWindow(value: unknown): NisarSubsetLogicalWindowV1 {
  const object = exactObject(value, [
    "component_id",
    "dataset_path",
    "frequency",
    "polarization",
    "swath",
    "line_start",
    "line_end_exclusive",
    "sample_start",
    "sample_end_exclusive",
    "source_shape",
    "expected_shape",
    "dtype",
    "chunk_shape",
    "margin_lines",
    "margin_samples",
    "mapping_confidence",
    "validity_mask_required",
  ]);
  return {
    component_id: boundedInteger(object.component_id, "component_id", 0, NISAR_SUBSET_MAX_COMPONENTS - 1),
    dataset_path: safeDatasetPath(object.dataset_path, "dataset_path"),
    frequency: enumValue(object.frequency, ["A", "B"] as const, "window.frequency"),
    polarization: stringValue(object.polarization, "window.polarization"),
    swath: safeText(object.swath, "window.swath", 128),
    line_start: nonNegativeInteger(object.line_start, "line_start"),
    line_end_exclusive: positiveInteger(object.line_end_exclusive, "line_end_exclusive"),
    sample_start: nonNegativeInteger(object.sample_start, "sample_start"),
    sample_end_exclusive: positiveInteger(object.sample_end_exclusive, "sample_end_exclusive"),
    source_shape: tuple2(object.source_shape, "source_shape"),
    expected_shape: tuple2(object.expected_shape, "expected_shape"),
    dtype: safeText(object.dtype, "dtype", 64),
    chunk_shape: tuple2(object.chunk_shape, "chunk_shape"),
    margin_lines: nonNegativeInteger(object.margin_lines, "margin_lines"),
    margin_samples: nonNegativeInteger(object.margin_samples, "margin_samples"),
    mapping_confidence: enumValue(object.mapping_confidence, ["conservative", "reference_bounded"] as const, "mapping_confidence"),
    validity_mask_required: booleanValue(object.validity_mask_required, "validity_mask_required"),
  };
}

function parseExpectedOutput(value: unknown): NisarSubsetExpectedOutputV1 {
  const object = exactObject(value, [
    "derived_artifact_id",
    "file_name",
    "estimated_output_bytes",
    "estimated_remote_bytes",
    "full_product_bytes",
  ]);
  return {
    derived_artifact_id: safeText(object.derived_artifact_id, "derived_artifact_id", NISAR_SUBSET_MAX_STRING_BYTES),
    file_name: derivedFileName(object.file_name, "expected_output.file_name"),
    estimated_output_bytes: positiveInteger(object.estimated_output_bytes, "estimated_output_bytes"),
    estimated_remote_bytes: positiveInteger(object.estimated_remote_bytes, "estimated_remote_bytes"),
    full_product_bytes: positiveInteger(object.full_product_bytes, "full_product_bytes"),
  };
}

function parseSourceValidator(value: unknown): NisarSubsetSourceValidatorV1 {
  const object = exactObject(value, ["kind", "value", "content_length"]);
  return {
    kind: enumValue(object.kind, ["strong_etag", "last_modified"] as const, "source_validator.kind"),
    value: safeText(object.value, "source_validator.value", 256),
    content_length: positiveInteger(object.content_length, "source_validator.content_length"),
  };
}

function parseRange(value: unknown): NisarSubsetByteRangeV1 {
  const object = exactObject(value, ["start", "end_exclusive", "expected_bytes", "received_bytes", "state"], ["sha256"]);
  return {
    start: nonNegativeInteger(object.start, "range.start"),
    end_exclusive: positiveInteger(object.end_exclusive, "range.end_exclusive"),
    expected_bytes: positiveInteger(object.expected_bytes, "range.expected_bytes"),
    received_bytes: nonNegativeInteger(object.received_bytes, "range.received_bytes"),
    ...(object.sha256 === undefined ? {} : {sha256: hash(object.sha256, "range.sha256")}),
    state: enumValue(object.state, ["PLANNED", "IN_PROGRESS", "COMPLETE", "FAILED"] as const, "range.state"),
  };
}

function parseExtent(value: unknown): NisarSubsetActualExtentV1 {
  const object = exactObject(value, ["line_start", "line_end_exclusive", "sample_start", "sample_end_exclusive", "component_count"]);
  return {
    line_start: nonNegativeInteger(object.line_start, "actual_extent.line_start"),
    line_end_exclusive: positiveInteger(object.line_end_exclusive, "actual_extent.line_end_exclusive"),
    sample_start: nonNegativeInteger(object.sample_start, "actual_extent.sample_start"),
    sample_end_exclusive: positiveInteger(object.sample_end_exclusive, "actual_extent.sample_end_exclusive"),
    component_count: boundedInteger(object.component_count, "actual_extent.component_count", 1, NISAR_SUBSET_MAX_COMPONENTS),
  };
}

function parseMask(value: unknown): NisarSubsetValidityMaskV1 {
  const object = exactObject(value, [
    "component_id", "dataset_path", "frequency", "swath", "line_start", "line_end_exclusive",
    "sample_start", "sample_end_exclusive", "shape", "sha256", "semantics",
  ]);
  return {
    component_id: boundedInteger(object.component_id, "mask.component_id", 0, NISAR_SUBSET_MAX_COMPONENTS - 1),
    dataset_path: safeMaskDatasetPath(object.dataset_path, "mask.dataset_path"),
    frequency: enumValue(object.frequency, ["A", "B"] as const, "mask.frequency"),
    swath: safeText(object.swath, "mask.swath", 128),
    line_start: nonNegativeInteger(object.line_start, "mask.line_start"),
    line_end_exclusive: positiveInteger(object.line_end_exclusive, "mask.line_end_exclusive"),
    sample_start: nonNegativeInteger(object.sample_start, "mask.sample_start"),
    sample_end_exclusive: positiveInteger(object.sample_end_exclusive, "mask.sample_end_exclusive"),
    shape: tuple2(object.shape, "mask.shape"),
    sha256: hash(object.sha256, "mask.sha256"),
    semantics: safeText(object.semantics, "mask.semantics", 128),
  };
}

function parseWriter(value: unknown): NisarSubsetWriterV1 {
  const object = exactObject(value, ["writer_id", "version"]);
  return {writer_id: safeText(object.writer_id, "writer.writer_id", NISAR_SUBSET_MAX_STRING_BYTES), version: positiveInteger(object.version, "writer.version")};
}

function parseValidator(value: unknown): NisarSubsetValidatorPolicyV1 {
  const object = exactObject(value, ["policy_id", "version", "product_specification_version"]);
  return {
    policy_id: safeText(object.policy_id, "validator.policy_id", NISAR_SUBSET_MAX_STRING_BYTES),
    version: positiveInteger(object.version, "validator.version"),
    product_specification_version: safeText(object.product_specification_version, "validator.product_specification_version", 64),
  };
}

function parseScienceValidation(value: unknown): NisarSubsetScienceValidationV1 {
  const object = exactObject(value, ["structure", "complex_values", "phase", "amplitude", "coordinates", "metadata", "provenance", "recovery"]);
  const parseCheck = (name: string): NisarSubsetValidationCheckV1 => {
    const check = exactObject(object[name], ["status", "evidence"]);
    return {
      status: enumValue(check.status, ["passed", "failed", "not_run"] as const, `${name}.status`),
      evidence: safeText(check.evidence, `${name}.evidence`, 512),
    };
  };
  return {
    structure: parseCheck("structure"),
    complex_values: parseCheck("complex_values"),
    phase: parseCheck("phase"),
    amplitude: parseCheck("amplitude"),
    coordinates: parseCheck("coordinates"),
    metadata: parseCheck("metadata"),
    provenance: parseCheck("provenance"),
    recovery: parseCheck("recovery"),
  };
}

function validateRequestManifest(manifest: NisarSubsetRequestManifestV1): void {
  _validateRequest(manifest.request);
  for (const window of manifest.mapping.windows) validateWindow(window, manifest.request);
  if (!manifest.mapping.windows.length || manifest.mapping.windows.length > NISAR_SUBSET_MAX_WINDOWS) {
    throw new ManifestContractError("NISAR_SUBSET_WINDOW_INVALID", "mapping.windows");
  }
  const covered = new Set<number>();
  const logicalIdentities = new Set<string>();
  const maskIdentities = new Set<string>();
  for (const window of manifest.mapping.windows) {
    if (window.component_id >= manifest.request.aoi_component_count) {
      throw new ManifestContractError("NISAR_SUBSET_WINDOW_INVALID", "component outside AOI");
    }
    covered.add(window.component_id);
    const logicalIdentity = JSON.stringify([
      window.component_id, window.dataset_path, window.frequency, window.polarization, window.swath,
      window.line_start, window.line_end_exclusive, window.sample_start, window.sample_end_exclusive,
    ]);
    if (logicalIdentities.has(logicalIdentity)) {
      throw new ManifestContractError("NISAR_SUBSET_WINDOW_INVALID", "duplicate logical window");
    }
    logicalIdentities.add(logicalIdentity);
    if (window.validity_mask_required) {
      const maskIdentity = JSON.stringify([
        window.component_id, window.frequency, window.swath, window.line_start,
        window.line_end_exclusive, window.sample_start, window.sample_end_exclusive,
      ]);
      if (maskIdentities.has(maskIdentity)) {
        throw new ManifestContractError("NISAR_SUBSET_WINDOW_INVALID", "duplicate mask identity");
      }
      maskIdentities.add(maskIdentity);
    }
  }
  for (let component = 0; component < manifest.request.aoi_component_count; component += 1) {
    if (!covered.has(component)) throw new ManifestContractError("NISAR_SUBSET_WINDOW_INVALID", "component coverage");
  }
  _validateExpectedOutput(manifest.expected_output, manifest.source.expected_size_bytes);
}

function validateTransferManifest(manifest: NisarSubsetTransferManifestV1): void {
  if (!manifest.ranges.length || manifest.ranges.length > NISAR_SUBSET_MAX_RANGES) {
    throw new ManifestContractError("NISAR_SUBSET_RANGE_INVALID", "range count");
  }
  let previousEnd = 0;
  let expectedTotal = 0;
  let receivedTotal = 0;
  for (let index = 0; index < manifest.ranges.length; index += 1) {
    const range = manifest.ranges[index];
    if (range.start >= range.end_exclusive || (index > 0 && range.start < previousEnd)) {
      throw new ManifestContractError("NISAR_SUBSET_RANGE_INVALID", "overlap or ordering");
    }
    const width = range.end_exclusive - range.start;
    if (range.expected_bytes !== width || range.received_bytes > width) {
      throw new ManifestContractError("NISAR_SUBSET_RANGE_INVALID", `range ${index} accounting`);
    }
    if (range.state === "COMPLETE" && (range.received_bytes !== width || !range.sha256)) {
      throw new ManifestContractError("NISAR_SUBSET_RANGE_INVALID", `range ${index} evidence`);
    }
    validateRangeState(range, index, width);
    expectedTotal += width;
    receivedTotal += range.received_bytes;
    previousEnd = range.end_exclusive;
  }
  if (expectedTotal !== manifest.expected_remote_bytes || receivedTotal !== manifest.received_bytes) {
    throw new ManifestContractError("NISAR_SUBSET_RANGE_INVALID", "manifest totals");
  }
  const createdAt = validateRfc3339("created_at", manifest.created_at);
  const updatedAt = validateRfc3339("updated_at", manifest.updated_at);
  if (updatedAt < createdAt) throw new ManifestContractError("NISAR_SUBSET_MANIFEST_INVALID", "updated_at precedes created_at");
  validateTransferState(manifest.state, manifest.ranges, manifest.expected_remote_bytes, manifest.received_bytes);
  if (manifest.source_validator.kind === "strong_etag" && (
    !manifest.source_validator.value.startsWith('"')
    || !manifest.source_validator.value.endsWith('"')
    || manifest.source_validator.value.startsWith("W/")
  )) {
    throw new ManifestContractError("NISAR_SUBSET_RANGE_INVALID", "strong_etag");
  }
  if (manifest.source_validator.content_length !== manifest.source_identity.expected_size_bytes) {
    throw new ManifestContractError("NISAR_SUBSET_IDENTITY_MISMATCH", "validator content length");
  }
}

function validateArtifactManifest(manifest: NisarSubsetArtifactManifestV1): void {
  if (!manifest.dataset_windows.length || manifest.dataset_windows.length > NISAR_SUBSET_MAX_WINDOWS) {
    throw new ManifestContractError("NISAR_SUBSET_WINDOW_INVALID", "dataset_windows");
  }
  for (const window of manifest.dataset_windows) validateWindow(window);
  if (manifest.validity_masks.length > NISAR_SUBSET_MAX_WINDOWS) {
    throw new ManifestContractError("NISAR_SUBSET_ARTIFACT_NOT_VALIDATED", "validity_masks");
  }
  for (const mask of manifest.validity_masks) {
    if (mask.dataset_path !== validityMaskDatasetPath(mask.frequency)
      || mask.line_start >= mask.line_end_exclusive
      || mask.sample_start >= mask.sample_end_exclusive
      || mask.shape[0] !== mask.line_end_exclusive - mask.line_start
      || mask.shape[1] !== mask.sample_end_exclusive - mask.sample_start) {
      throw new ManifestContractError("NISAR_SUBSET_WINDOW_INVALID", "mask identity");
    }
  }
  for (const [name, check] of Object.entries(manifest.science_validation)) {
    if (check.status !== "passed") throw new ManifestContractError("NISAR_SUBSET_ARTIFACT_NOT_VALIDATED", `science check ${name}`);
  }
  if (manifest.validator.product_specification_version !== manifest.source_product_identity.product_specification_version) {
    throw new ManifestContractError("NISAR_SUBSET_IDENTITY_MISMATCH", "validator specification version");
  }
  _validateByteTotals(manifest.remote_bytes, manifest.full_product_bytes, "artifact");
}

function validateWindow(window: NisarSubsetLogicalWindowV1, request?: NisarSubsetRequestV1): void {
  _validatePolarization(window.frequency, window.polarization);
  if (window.dataset_path !== scienceDatasetPath(window.frequency, window.polarization)) {
    throw new ManifestContractError("NISAR_SUBSET_WINDOW_INVALID", "science dataset path");
  }
  if (request && (
    window.frequency !== request.frequency
    || window.polarization !== request.polarization
    || window.swath !== request.swath
  )) throw new ManifestContractError("NISAR_SUBSET_IDENTITY_MISMATCH", "window science identity");
  if (
    window.line_start >= window.line_end_exclusive
    || window.sample_start >= window.sample_end_exclusive
    || window.line_end_exclusive > window.source_shape[0]
    || window.sample_end_exclusive > window.source_shape[1]
    || window.expected_shape[0] !== window.line_end_exclusive - window.line_start
    || window.expected_shape[1] !== window.sample_end_exclusive - window.sample_start
  ) throw new ManifestContractError("NISAR_SUBSET_WINDOW_INVALID", "window bounds");
}

function _validateRequest(request: NisarSubsetRequestV1): void {
  _validatePolarization(request.frequency, request.polarization);
  const upper = request.swath.toUpperCase();
  if (["SSAR", "S-BAND", "BHOONIDHI"].some((value) => upper.includes(value))) {
    throw new ManifestContractError("NISAR_SUBSET_UNSUPPORTED_SCOPE", "swath");
  }
}

function _validateExpectedOutput(output: NisarSubsetExpectedOutputV1, sourceSize: number): void {
  if (output.full_product_bytes !== sourceSize) throw new ManifestContractError("NISAR_SUBSET_IDENTITY_MISMATCH", "expected output bytes");
  _validateByteTotals(output.estimated_remote_bytes, output.full_product_bytes, "expected output");
}

function _validatePolarization(frequency: Frequency, polarization: string): void {
  const allowed = frequency === "A" ? ["HH", "HV"] : ["HH", "HV", "VV"];
  if (!allowed.includes(polarization)) throw new ManifestContractError("NISAR_SUBSET_UNSUPPORTED_SCOPE", "polarization");
}

function scienceDatasetPath(frequency: Frequency, polarization: string): string {
  return `/science/LSAR/RSLC/swaths/frequency${frequency}/${polarization}`;
}

function validityMaskDatasetPath(frequency: Frequency): string {
  return `/science/LSAR/RSLC/swaths/frequency${frequency}/${NISAR_SUBSET_VALIDITY_MASK_SUFFIX}`;
}

function _validateByteTotals(remote: number, full: number, label: string): void {
  if (full <= 0 || remote <= 0 || remote > full || !Number.isSafeInteger(remote) || !Number.isSafeInteger(full)) {
    throw new ManifestContractError("NISAR_SUBSET_RANGE_INVALID", `${label} byte totals`);
  }
}

function validateRangeState(range: NisarSubsetByteRangeV1, index: number, width: number): void {
  const valid = {
    PLANNED: range.received_bytes === 0 && range.sha256 === undefined,
    IN_PROGRESS: range.received_bytes > 0 && range.received_bytes < width && range.sha256 === undefined,
    COMPLETE: range.received_bytes === width && range.sha256 !== undefined,
    FAILED: range.received_bytes < width && range.sha256 === undefined,
  }[range.state];
  if (!valid) throw new ManifestContractError("NISAR_SUBSET_RANGE_INVALID", `range ${index} state evidence`);
}

function validateTransferState(
  state: TransferState,
  ranges: NisarSubsetByteRangeV1[],
  expectedRemoteBytes: number,
  receivedBytes: number,
): void {
  const allComplete = ranges.every((range) => range.state === "COMPLETE");
  const anyFailed = ranges.some((range) => range.state === "FAILED");
  const anyStarted = ranges.some((range) => range.state !== "PLANNED");
  const valid = {
    PLANNED: receivedBytes === 0 && ranges.every((range) => range.state === "PLANNED"),
    IN_PROGRESS: receivedBytes < expectedRemoteBytes && !allComplete && anyStarted,
    INTERRUPTED: receivedBytes < expectedRemoteBytes && !allComplete,
    SOURCE_CHANGED: receivedBytes < expectedRemoteBytes && !allComplete,
    RANGES_COMPLETE: receivedBytes === expectedRemoteBytes && allComplete,
    FAILED: receivedBytes < expectedRemoteBytes && !allComplete && anyFailed,
  }[state];
  if (!valid) throw new ManifestContractError("NISAR_SUBSET_RANGE_INVALID", "transfer state evidence");
}

function validateRfc3339(label: string, value: string): bigint {
  if (!/^\d{4}-\d{2}-\d{2}T\d{2}:\d{2}:\d{2}(?:\.\d{1,9})?(?:Z|[+-]\d{2}:\d{2})$/.test(value)) {
    throw new ManifestContractError("NISAR_SUBSET_MANIFEST_INVALID", `${label} is not RFC3339`);
  }
  const match = value.match(/^(\d{4})-(\d{2})-(\d{2})T(\d{2}):(\d{2}):(\d{2})/);
  if (!match) throw new ManifestContractError("NISAR_SUBSET_MANIFEST_INVALID", `${label} is not RFC3339`);
  const year = Number(match[1]);
  const month = Number(match[2]);
  const day = Number(match[3]);
  const hour = Number(match[4]);
  const minute = Number(match[5]);
  const second = Number(match[6]);
  const monthLengths = [31, isLeapYear(year) ? 29 : 28, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31];
  const lastDay = monthLengths[month - 1] ?? 0;
  if (year === 0 || month < 1 || month > 12 || day < 1 || day > lastDay || hour > 23 || minute > 59 || second > 59) {
    throw new ManifestContractError("NISAR_SUBSET_MANIFEST_INVALID", `${label} is not RFC3339`);
  }
  const offset = value.match(/(?:Z|([+-])(\d{2}):(\d{2}))$/);
  if (!offset || (offset[2] !== undefined && (Number(offset[2]) > 23 || Number(offset[3]) > 59))) {
    throw new ManifestContractError("NISAR_SUBSET_MANIFEST_INVALID", `${label} is not RFC3339`);
  }
  const offsetMinutes = offset[1] === undefined
    ? 0
    : (offset[1] === "+" ? 1 : -1) * (Number(offset[2]) * 60 + Number(offset[3]));
  const fraction = value.match(/\.(\d{1,9})/)?.[1] ?? "";
  const nanoseconds = BigInt((fraction + "000000000").slice(0, 9));
  const previousYear = BigInt(year - 1);
  const daysBeforeYear = previousYear * 365n
    + previousYear / 4n
    - previousYear / 100n
    + previousYear / 400n;
  const daysBeforeMonth = monthLengths.slice(0, month - 1).reduce((sum, length) => sum + length, 0);
  const localSeconds = (daysBeforeYear + BigInt(daysBeforeMonth + day - 1)) * 86_400n
    + BigInt(hour * 3_600 + minute * 60 + second);
  return (localSeconds - BigInt(offsetMinutes * 60)) * 1_000_000_000n + nanoseconds;
}

function isLeapYear(year: number): boolean {
  return year % 400 === 0 || (year % 4 === 0 && year % 100 !== 0);
}

export async function validateTransferAgainstRequest(
  requestValue: unknown,
  transferValue: unknown,
): Promise<void> {
  const request = parseRequestManifest(requestValue);
  const transfer = parseTransferManifest(transferValue);
  if (transfer.request_manifest_sha256 !== await canonicalSha256(request)) {
    throw new ManifestContractError("NISAR_SUBSET_IDENTITY_MISMATCH", "transfer request digest");
  }
  if (!sameJson(transfer.source_identity, request.source)) {
    throw new ManifestContractError("NISAR_SUBSET_IDENTITY_MISMATCH", "transfer source identity");
  }
  if (transfer.expected_remote_bytes !== request.expected_output.estimated_remote_bytes) {
    throw new ManifestContractError("NISAR_SUBSET_RANGE_INVALID", "transfer remote bytes");
  }
}

export async function validateArtifactAgainstRequestAndTransfer(
  requestValue: unknown,
  transferValue: unknown,
  artifactValue: unknown,
): Promise<void> {
  const request = parseRequestManifest(requestValue);
  const transfer = parseTransferManifest(transferValue);
  const artifact = parseArtifactManifest(artifactValue);
  await validateTransferAgainstRequest(request, transfer);
  if (artifact.request_manifest_sha256 !== await canonicalSha256(request)) {
    throw new ManifestContractError("NISAR_SUBSET_IDENTITY_MISMATCH", "artifact request digest");
  }
  if (artifact.transfer_manifest_sha256 !== await canonicalSha256(transfer)) {
    throw new ManifestContractError("NISAR_SUBSET_IDENTITY_MISMATCH", "artifact transfer digest");
  }
  if (!sameJson(artifact.source_product_identity, request.source) || !sameJson(artifact.source_product_identity, transfer.source_identity)) {
    throw new ManifestContractError("NISAR_SUBSET_IDENTITY_MISMATCH", "artifact source identity");
  }
  if (transfer.state !== "RANGES_COMPLETE"
    || transfer.received_bytes !== transfer.expected_remote_bytes
    || transfer.ranges.some((range) => range.state !== "COMPLETE" || range.sha256 === undefined)) {
    throw new ManifestContractError("NISAR_SUBSET_ARTIFACT_NOT_VALIDATED", "transfer is incomplete");
  }
  if (artifact.remote_bytes !== transfer.received_bytes || artifact.remote_bytes !== request.expected_output.estimated_remote_bytes) {
    throw new ManifestContractError("NISAR_SUBSET_RANGE_INVALID", "artifact remote bytes");
  }
  if (artifact.full_product_bytes !== request.source.expected_size_bytes || artifact.full_product_bytes !== request.expected_output.full_product_bytes) {
    throw new ManifestContractError("NISAR_SUBSET_IDENTITY_MISMATCH", "artifact full product bytes");
  }
  if (artifact.artifact_id !== request.expected_output.derived_artifact_id
    || artifact.artifact_file_name !== request.expected_output.file_name) {
    throw new ManifestContractError("NISAR_SUBSET_IDENTITY_MISMATCH", "artifact output identity");
  }
  validateArtifactWindowsAndMasks(artifact, request);
}

function sameJson(left: unknown, right: unknown): boolean {
  return JSON.stringify(sortJson(left)) === JSON.stringify(sortJson(right));
}

function validateArtifactWindowsAndMasks(
  artifact: NisarSubsetArtifactManifestV1,
  request: NisarSubsetRequestManifestV1,
): void {
  const requestWindows = [...request.mapping.windows].sort(compareWindowValues);
  const artifactWindows = [...artifact.dataset_windows].sort(compareWindowValues);
  if (!sameJson(requestWindows, artifactWindows)) {
    throw new ManifestContractError("NISAR_SUBSET_IDENTITY_MISMATCH", "artifact windows");
  }
  if (artifact.actual_extent.component_count !== request.request.aoi_component_count) {
    throw new ManifestContractError("NISAR_SUBSET_WINDOW_INVALID", "artifact component count");
  }
  const components = new Set(requestWindows.map((window) => window.component_id));
  for (let index = 0; index < request.request.aoi_component_count; index += 1) {
    if (!components.has(index)) throw new ManifestContractError("NISAR_SUBSET_WINDOW_INVALID", "artifact component set");
  }
  const extent = {
    line_start: Math.min(...requestWindows.map((window) => window.line_start)),
    line_end_exclusive: Math.max(...requestWindows.map((window) => window.line_end_exclusive)),
    sample_start: Math.min(...requestWindows.map((window) => window.sample_start)),
    sample_end_exclusive: Math.max(...requestWindows.map((window) => window.sample_end_exclusive)),
  };
  if (!sameJson(extent, {
    line_start: artifact.actual_extent.line_start,
    line_end_exclusive: artifact.actual_extent.line_end_exclusive,
    sample_start: artifact.actual_extent.sample_start,
    sample_end_exclusive: artifact.actual_extent.sample_end_exclusive,
  })) {
    throw new ManifestContractError("NISAR_SUBSET_WINDOW_INVALID", "artifact extent");
  }
  const requiredKeys = requestWindows
    .filter((window) => window.validity_mask_required)
    .map((window) => maskIdentity(window, validityMaskDatasetPath(window.frequency)));
  const artifactKeys = artifact.validity_masks.map((mask) => maskIdentity(mask));
  const sortedRequiredKeys = [...requiredKeys].sort(compareMaskIdentity);
  const sortedArtifactKeys = [...artifactKeys].sort(compareMaskIdentity);
  if (hasDuplicateMaskIdentities(sortedRequiredKeys)
    || hasDuplicateMaskIdentities(sortedArtifactKeys)
    || sortedArtifactKeys.length !== sortedRequiredKeys.length
    || sortedArtifactKeys.some((key, index) => compareMaskIdentity(key, sortedRequiredKeys[index]) !== 0)) {
    throw new ManifestContractError("NISAR_SUBSET_ARTIFACT_NOT_VALIDATED", "missing or unrelated masks");
  }
}

type MaskIdentity = readonly [number, string, string, string, number, number, number, number, number, number];

function maskIdentity(
  value: NisarSubsetLogicalWindowV1 | NisarSubsetValidityMaskV1,
  datasetPath = value.dataset_path,
): MaskIdentity {
  const shape = "shape" in value ? value.shape : value.expected_shape;
  return [
    value.component_id, datasetPath, value.frequency, value.swath,
    value.line_start, value.line_end_exclusive, value.sample_start, value.sample_end_exclusive,
    shape[0], shape[1],
  ];
}

function compareMaskIdentity(left: MaskIdentity, right: MaskIdentity): number {
  return firstDifference([
    compareNumber(left[0], right[0]),
    compareString(left[1], right[1]),
    compareString(left[2], right[2]),
    compareString(left[3], right[3]),
    compareNumber(left[4], right[4]),
    compareNumber(left[5], right[5]),
    compareNumber(left[6], right[6]),
    compareNumber(left[7], right[7]),
    compareNumber(left[8], right[8]),
    compareNumber(left[9], right[9]),
  ]);
}

function hasDuplicateMaskIdentities(values: readonly MaskIdentity[]): boolean {
  return values.some((value, index) => index > 0 && compareMaskIdentity(value, values[index - 1]) === 0);
}

function exactObject(value: unknown, required: readonly string[], optional: readonly string[] = []): ObjectValue {
  if (value === null || typeof value !== "object" || Array.isArray(value)) {
    throw new ManifestContractError("NISAR_SUBSET_MANIFEST_INVALID", "object expected");
  }
  const object = value as ObjectValue;
  const allowed = new Set([...required, ...optional]);
  for (const key of Object.keys(object)) {
    if (!allowed.has(key)) throw new ManifestContractError("NISAR_SUBSET_MANIFEST_INVALID", `unknown field ${key}`);
  }
  for (const key of required) {
    if (!(key in object)) throw new ManifestContractError("NISAR_SUBSET_MANIFEST_INVALID", `missing field ${key}`);
  }
  return object;
}

function array<T>(value: unknown, label: string, parser: (value: unknown) => T): T[] {
  if (!Array.isArray(value)) throw new ManifestContractError("NISAR_SUBSET_MANIFEST_INVALID", label);
  return value.map(parser);
}

function parseJson(value: string, maximum: number): unknown {
  if (new TextEncoder().encode(value).length > maximum) {
    throw new ManifestContractError("NISAR_SUBSET_MANIFEST_TOO_LARGE", String(maximum));
  }
  try {
    return JSON.parse(value) as unknown;
  } catch {
    throw new ManifestContractError("NISAR_SUBSET_MANIFEST_INVALID", "invalid JSON");
  }
}

function cloneJson(value: unknown): unknown {
  return JSON.parse(JSON.stringify(value)) as unknown;
}

function stringValue(value: unknown, label: string): string {
  if (typeof value !== "string") throw new ManifestContractError("NISAR_SUBSET_MANIFEST_INVALID", label);
  return value;
}

function safeText(value: unknown, label: string, maximum: number): string {
  const text = stringValue(value, label);
  if (!text || new TextEncoder().encode(text).length > maximum || [...text].some((character) => {
    const code = character.codePointAt(0) ?? 0;
    return code < 0x20 || (code >= 0x7f && code <= 0x9f);
  })) {
    throw new ManifestContractError("NISAR_SUBSET_MANIFEST_INVALID", label);
  }
  const lower = text.toLowerCase();
  if (["http://", "https://", "authorization", "cookie", "password", "secret", "token", ".netrc", "signed_url", "signedurl"].some((value) => lower.includes(value))) {
    throw new ManifestContractError("NISAR_SUBSET_SENSITIVE_FIELD", label);
  }
  return text;
}

function safeDatasetPath(value: unknown, label: string): string {
  const path = safeText(value, label, 512);
  if (!path.startsWith("/") || path.includes("..") || path.includes("\\")) {
    throw new ManifestContractError("NISAR_SUBSET_WINDOW_INVALID", label);
  }
  return path;
}

function safeMaskDatasetPath(value: unknown, label: string): string {
  const path = safeDatasetPath(value, label);
  if (!path.endsWith(`/${NISAR_SUBSET_VALIDITY_MASK_SUFFIX}`)) {
    throw new ManifestContractError("NISAR_SUBSET_WINDOW_INVALID", label);
  }
  return path;
}

function stringNumber(value: unknown, label: string): number {
  if (typeof value !== "number" || !Number.isFinite(value) || !Number.isSafeInteger(value)) {
    throw new ManifestContractError("NISAR_SUBSET_MANIFEST_INVALID", label);
  }
  return value;
}

function positiveInteger(value: unknown, label: string): number {
  const result = stringNumber(value, label);
  if (result <= 0) throw new ManifestContractError("NISAR_SUBSET_MANIFEST_INVALID", label);
  return result;
}

function nonNegativeInteger(value: unknown, label: string): number {
  const result = stringNumber(value, label);
  if (result < 0) throw new ManifestContractError("NISAR_SUBSET_MANIFEST_INVALID", label);
  return result;
}

function boundedInteger(value: unknown, label: string, minimum: number, maximum: number): number {
  const result = stringNumber(value, label);
  if (result < minimum || result > maximum) throw new ManifestContractError("NISAR_SUBSET_MANIFEST_INVALID", label);
  return result;
}

function booleanValue(value: unknown, label: string): boolean {
  if (typeof value !== "boolean") throw new ManifestContractError("NISAR_SUBSET_MANIFEST_INVALID", label);
  return value;
}

function tuple2(value: unknown, label: string): [number, number] {
  if (!Array.isArray(value) || value.length !== 2) throw new ManifestContractError("NISAR_SUBSET_MANIFEST_INVALID", label);
  return [positiveInteger(value[0], `${label}[0]`), positiveInteger(value[1], `${label}[1]`)];
}

function hash(value: unknown, label: string): string {
  const digest = stringValue(value, label);
  if (!/^[0-9a-f]{64}$/.test(digest)) throw new ManifestContractError("NISAR_SUBSET_MANIFEST_INVALID", label);
  return digest;
}

function derivedFileName(value: unknown, label: string): string {
  const name = safeText(value, label, NISAR_SUBSET_MAX_STRING_BYTES);
  const lower = name.toLowerCase();
  if (name.includes("/") || name.includes("\\") || name.endsWith(".part") || (!(lower.includes("subset") || lower.includes("derived")))) {
    throw new ManifestContractError("NISAR_SUBSET_IDENTITY_MISMATCH", label);
  }
  return name;
}

function literal<T>(object: ObjectValue, label: string, expected: T): T {
  if (object[label] !== expected) throw new ManifestContractError("NISAR_SUBSET_UNSUPPORTED_SCOPE", label);
  return expected;
}

function enumValue<const T extends readonly string[]>(value: unknown, allowed: T, label: string): T[number] {
  if (typeof value !== "string" || !(allowed as readonly string[]).includes(value)) throw new ManifestContractError("NISAR_SUBSET_MANIFEST_INVALID", label);
  return value as T[number];
}

function compareWindowValues(left: unknown, right: unknown): number {
  const a = left as NisarSubsetLogicalWindowV1;
  const b = right as NisarSubsetLogicalWindowV1;
  return firstDifference([
    compareNumber(a.component_id, b.component_id),
    compareString(a.dataset_path, b.dataset_path),
    compareString(a.frequency, b.frequency),
    compareString(a.polarization, b.polarization),
    compareString(a.swath, b.swath),
    compareString(a.dtype, b.dtype),
    compareNumber(a.line_start, b.line_start),
    compareNumber(a.line_end_exclusive, b.line_end_exclusive),
    compareNumber(a.sample_start, b.sample_start),
    compareNumber(a.sample_end_exclusive, b.sample_end_exclusive),
    compareNumber(a.source_shape[0], b.source_shape[0]),
    compareNumber(a.source_shape[1], b.source_shape[1]),
    compareNumber(a.expected_shape[0], b.expected_shape[0]),
    compareNumber(a.expected_shape[1], b.expected_shape[1]),
    compareNumber(a.chunk_shape[0], b.chunk_shape[0]),
    compareNumber(a.chunk_shape[1], b.chunk_shape[1]),
    compareNumber(a.margin_lines, b.margin_lines),
    compareNumber(a.margin_samples, b.margin_samples),
    compareString(a.mapping_confidence, b.mapping_confidence),
    compareBoolean(a.validity_mask_required, b.validity_mask_required),
  ]);
}

function compareRangeValues(left: unknown, right: unknown): number {
  const a = left as NisarSubsetByteRangeV1;
  const b = right as NisarSubsetByteRangeV1;
  return firstDifference([
    compareNumber(a.start, b.start),
    compareNumber(a.end_exclusive, b.end_exclusive),
    compareNumber(a.expected_bytes, b.expected_bytes),
    compareNumber(a.received_bytes, b.received_bytes),
    compareString(a.sha256 ?? "", b.sha256 ?? ""),
    compareString(a.state, b.state),
  ]);
}

function compareMaskValues(left: unknown, right: unknown): number {
  const a = left as NisarSubsetValidityMaskV1;
  const b = right as NisarSubsetValidityMaskV1;
  return firstDifference([
    compareString(a.dataset_path, b.dataset_path),
    compareNumber(a.component_id, b.component_id),
    compareString(a.frequency, b.frequency),
    compareString(a.swath, b.swath),
    compareNumber(a.line_start, b.line_start),
    compareNumber(a.line_end_exclusive, b.line_end_exclusive),
    compareNumber(a.sample_start, b.sample_start),
    compareNumber(a.sample_end_exclusive, b.sample_end_exclusive),
    compareNumber(a.shape[0], b.shape[0]),
    compareNumber(a.shape[1], b.shape[1]),
    compareString(a.sha256, b.sha256),
    compareString(a.semantics, b.semantics),
  ]);
}

function firstDifference(values: number[]): number {
  return values.find((value) => value !== 0) ?? 0;
}

function compareNumber(left: number, right: number): number {
  return left < right ? -1 : left > right ? 1 : 0;
}

function compareString(left: string, right: string): number {
  const leftBytes = new TextEncoder().encode(left);
  const rightBytes = new TextEncoder().encode(right);
  const length = Math.min(leftBytes.length, rightBytes.length);
  for (let index = 0; index < length; index += 1) {
    if (leftBytes[index] !== rightBytes[index]) return leftBytes[index] < rightBytes[index] ? -1 : 1;
  }
  return compareNumber(leftBytes.length, rightBytes.length);
}

function compareBoolean(left: boolean, right: boolean): number {
  return Number(left) - Number(right);
}

function sortJson(value: unknown): unknown {
  if (Array.isArray(value)) return value.map(sortJson);
  if (value !== null && typeof value === "object") {
    const object = value as ObjectValue;
    return Object.fromEntries(Object.keys(object).sort().map((key) => [key, sortJson(object[key])]));
  }
  return value;
}
