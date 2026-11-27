import assert from "node:assert/strict";
import {readFileSync} from "node:fs";
import {dirname, resolve} from "node:path";
import {fileURLToPath} from "node:url";
import {test} from "node:test";

import {
  NISAR_REMOTE_SUBSET_METADATA_KEY,
  NISAR_SUBSET_MAX_ARTIFACT_CANONICAL_JSON_BYTES,
  NISAR_SUBSET_MAX_RANGES,
  NISAR_SUBSET_MAX_TRANSFER_CANONICAL_JSON_BYTES,
  ManifestContractError,
  canonicalJson,
  canonicalSha256,
  embedRequestManifest,
  parseArtifactManifest,
  parseRequestManifest,
  parseRequestManifestJson,
  parseRequestManifestFromMetadata,
  parseTransferManifest,
  validateArtifactAgainstRequestAndTransfer,
  validateRequestAgainstSnapshotItem,
  validateTransferAgainstRequest,
} from "./nisarSubsetContracts.ts";

const root = resolve(dirname(fileURLToPath(import.meta.url)), "../../../");
const fixture = (name: string): unknown => JSON.parse(
  readFileSync(resolve(root, "docs/fixtures/asf-nisar-subset-p1", name), "utf8"),
);
const clone = <T>(value: T): T => JSON.parse(JSON.stringify(value)) as T;

function materializeInvalidCase(caseValue: any): unknown {
  return fixture(caseValue.fixture);
}

test("offline planner fixture passes the TypeScript P1.0 contract", async () => {
  const manifest = parseRequestManifest(fixture("planner-request-v1.json"));
  assert.equal(
    await canonicalSha256(manifest),
    "5d70c55ddf84cda9032a77b1f2a2437f3bb6aee84b038056effe943a77b76df4",
  );
});

function buildParityCase(builder: string): unknown {
  const base = clone(fixture("request-manifest-v1.json")) as any;
  const template = clone(base.mapping.windows[0]);
  if (builder === "two_windows_numeric" || builder === "two_windows_numeric_reversed") {
    const first = {...clone(template), line_start: 2, line_end_exclusive: 3, expected_shape: [1, 512]};
    const second = {...clone(template), line_start: 10, line_end_exclusive: 11, expected_shape: [1, 512]};
    base.mapping.windows = builder === "two_windows_numeric" ? [second, first] : [first, second];
    return base;
  }
  if (builder === "ratio_equal_one") {
    base.expected_output.estimated_remote_bytes = 100_000_000;
    base.expected_output.estimated_output_bytes = 1;
    return base;
  }
  if (builder === "ratio_very_small") {
    base.expected_output.estimated_remote_bytes = 1;
    base.expected_output.estimated_output_bytes = 1;
    return base;
  }
  if (builder === "near_max_safe_integer") {
    base.source.expected_size_bytes = Number.MAX_SAFE_INTEGER;
    base.expected_output.full_product_bytes = Number.MAX_SAFE_INTEGER;
    base.expected_output.estimated_remote_bytes = Number.MAX_SAFE_INTEGER - 1;
    return base;
  }
  if (builder === "same_prefix_different_end") {
    const first = {...clone(template), line_start: 2, line_end_exclusive: 3, expected_shape: [1, 512]};
    const second = {...clone(first), line_end_exclusive: 4, expected_shape: [2, 512]};
    base.mapping.windows = [second, first];
    return base;
  }
  if (builder === "unicode_bmp_supplementary_order") {
    const first = {...clone(template), dtype: "\uFFFF", line_start: 2, line_end_exclusive: 3, expected_shape: [1, 512]};
    const second = {...clone(template), dtype: "\u{10000}", line_start: 10, line_end_exclusive: 11, expected_shape: [1, 512]};
    base.mapping.windows = [second, first];
    return base;
  }
  if (builder === "oversized_request") {
    const swath = "s".repeat(128);
    base.request.swath = swath;
    base.request.aoi_component_count = 64;
    base.mapping.windows = Array.from({length: 64}, (_, component_id) => ({
      ...clone(template),
      component_id,
      swath,
      dtype: "d".repeat(64),
    }));
    return base;
  }
  if (builder === "invalid_mapping_algorithm") {
    base.mapping.algorithm = "other_algorithm_v1";
    return base;
  }
  if (builder === "invalid_mapping_version") {
    base.mapping.algorithm_version = 2;
    return base;
  }
  if (builder === "negative_margin") {
    base.mapping.windows[0].margin_lines = -1;
    return base;
  }
  const artifact = clone(fixture("artifact-manifest-v1.json")) as any;
  if (builder === "artifact_invalid_polarization") artifact.dataset_windows[0].polarization = "VV";
  else if (builder === "mask_path_backslash") artifact.validity_masks[0].dataset_path = "\\science\\inputDataExceptionMask";
  else if (builder === "source_validator_specification_mismatch") artifact.validator.product_specification_version = "1.5.1";
  else throw new Error(`unknown parity builder ${builder}`);
  return artifact;
}

test("all three manifests validate and match the shared canonical digests", async () => {
  const request = parseRequestManifest(fixture("request-manifest-v1.json"));
  const transfer = parseTransferManifest(fixture("transfer-manifest-v1.json"));
  const artifact = parseArtifactManifest(fixture("artifact-manifest-v1.json"));

  assert.ok(canonicalJson(request).length < 32 * 1024);
  assert.equal(await canonicalSha256(request), "2f276343dbf74fd2b92ad60ed0d34e41eb50b25d861be682579880fd2097199b");
  assert.equal(await canonicalSha256(transfer), "2e935cec7f6beb49b18b3fd51aea286a99307f5a55d9211b8b3dc4a680947adf");
  assert.equal(await canonicalSha256(artifact), "247d13e7ccf9f94efda3bca3e110512b2cbff1c3140200008a21fc489aa99977");
});

test("unknown fields, sensitive keys, and unsupported scope fail closed", () => {
  const unknown = fixture("request-manifest-v1.json") as Record<string, unknown>;
  unknown.unknown_field = true;
  assert.throws(() => parseRequestManifest(unknown), ManifestContractError);

  const sensitive = fixture("request-manifest-v1.json") as Record<string, any>;
  sensitive.source.token = "not-a-secret";
  assert.throws(() => parseRequestManifest(sensitive), ManifestContractError);

  const wrongAuthority = fixture("request-manifest-v1.json") as Record<string, unknown>;
  wrongAuthority.output_authority = "provider_generated";
  assert.throws(() => parseRequestManifest(wrongAuthority), ManifestContractError);

  const fullDownload = fixture("request-manifest-v1.json") as Record<string, any>;
  fullDownload.request.subset_method = "full_download_crop";
  assert.throws(() => parseRequestManifest(fullDownload), ManifestContractError);

  const coercedNumber = fixture("request-manifest-v1.json") as Record<string, any>;
  coercedNumber.source.expected_size_bytes = "100000000";
  assert.throws(() => parseRequestManifest(coercedNumber), ManifestContractError);
});

test("shared invalid fixture index executes in TypeScript", () => {
  const cases = JSON.parse(readFileSync(resolve(root, "docs/fixtures/asf-nisar-subset-p1/invalid/cases.json"), "utf8")) as any[];
  for (const caseValue of cases) {
    const value = materializeInvalidCase(caseValue);
    const parser = {
      request: parseRequestManifest,
      transfer: parseTransferManifest,
      artifact: parseArtifactManifest,
    }[caseValue.target as "request" | "transfer" | "artifact"];
    assert.throws(() => parser(value), ManifestContractError, `invalid fixture did not fail: ${caseValue.case}`);
  }
});

test("request manifest uses one namespaced task metadata envelope", () => {
  const request = parseRequestManifest(fixture("request-manifest-v1.json"));
  const metadata: Record<string, unknown> = {ordinary_field: "preserved"};
  embedRequestManifest(metadata, request);
  assert.deepEqual(parseRequestManifest(metadata[NISAR_REMOTE_SUBSET_METADATA_KEY]), request);
  assert.throws(() => parseRequestManifestFromMetadata({}), /NISAR_SUBSET_MANIFEST_MISSING/);
});

test("window, range, and artifact lifecycle failures are rejected", () => {
  const window = fixture("request-manifest-v1.json") as Record<string, any>;
  window.mapping.windows[0].line_end_exclusive = 1001;
  assert.throws(() => parseRequestManifest(window), ManifestContractError);

  const mixedFrequency = fixture("request-manifest-v1.json") as Record<string, any>;
  mixedFrequency.mapping.windows[0].frequency = "B";
  assert.throws(() => parseRequestManifest(mixedFrequency), ManifestContractError);

  const overlapping = fixture("transfer-manifest-v1.json") as Record<string, any>;
  overlapping.ranges[1].start = 512;
  assert.throws(() => parseTransferManifest(overlapping), ManifestContractError);

  const falseComplete = fixture("artifact-manifest-v1.json") as Record<string, any>;
  falseComplete.complete = false;
  assert.throws(() => parseArtifactManifest(falseComplete), ManifestContractError);
});

test("transfer and artifact contracts match request identity and science extent", async () => {
  const request = parseRequestManifest(fixture("request-manifest-v1.json"));
  const transfer = parseTransferManifest(fixture("transfer-manifest-v1.json"));
  const artifact = parseArtifactManifest(fixture("artifact-manifest-v1.json"));

  await validateTransferAgainstRequest(request, transfer);
  await validateArtifactAgainstRequestAndTransfer(request, transfer, artifact);

  const changedTransfer = clone(fixture("transfer-manifest-v1.json")) as Record<string, any>;
  changedTransfer.source_identity.provider_product_id = "different-product";
  const changedTransferParsed = parseTransferManifest(changedTransfer);
  await assert.rejects(() => validateTransferAgainstRequest(request, changedTransferParsed), ManifestContractError);

  const changedArtifact = clone(fixture("artifact-manifest-v1.json")) as Record<string, any>;
  changedArtifact.dataset_windows[0].polarization = "VV";
  assert.throws(() => parseArtifactManifest(changedArtifact), ManifestContractError);

  const unrelatedMask = clone(fixture("artifact-manifest-v1.json")) as Record<string, any>;
  unrelatedMask.validity_masks[0].dataset_path = "/science/LSAR/RSLC/swaths/frequencyA/HH/inputDataExceptionMask";
  assert.throws(() => parseArtifactManifest(unrelatedMask), ManifestContractError);

  const mismatchedSpecification = clone(fixture("artifact-manifest-v1.json")) as Record<string, any>;
  mismatchedSpecification.validator.product_specification_version = "1.5.1";
  assert.throws(() => parseArtifactManifest(mismatchedSpecification), ManifestContractError);
});

test("canonical limits numeric order ratios and safe integer bound are shared contracts", () => {
  const numeric = clone(fixture("request-manifest-v1.json")) as Record<string, any>;
  const template = clone(numeric.mapping.windows[0]);
  const windowTwo = {...template, line_start: 2, line_end_exclusive: 3, expected_shape: [1, 512]};
  const windowTen = {...template, line_start: 10, line_end_exclusive: 11, expected_shape: [1, 512]};
  numeric.mapping.windows = [windowTen, windowTwo];
  const manifest = parseRequestManifest(numeric);
  const rendered = canonicalJson(manifest);
  assert.ok(rendered.indexOf("\"line_start\":2") < rendered.indexOf("\"line_start\":10"));

  const samePrefix = clone(numeric);
  const first = {...template, line_start: 2, line_end_exclusive: 3, expected_shape: [1, 512]};
  const second = {...first, line_end_exclusive: 4, expected_shape: [2, 512]};
  samePrefix.mapping.windows = [second, first];
  const firstOrder = parseRequestManifest(samePrefix);
  const reversedOrder = {...firstOrder, mapping: {...firstOrder.mapping, windows: [...firstOrder.mapping.windows].reverse()}};
  assert.equal(canonicalJson(firstOrder), canonicalJson(reversedOrder));

  const equalRatio = clone(fixture("request-manifest-v1.json")) as Record<string, any>;
  equalRatio.expected_output.estimated_remote_bytes = 100_000_000;
  equalRatio.expected_output.estimated_output_bytes = 1;
  parseRequestManifest(equalRatio);

  const tinyRatio = clone(fixture("request-manifest-v1.json")) as Record<string, any>;
  tinyRatio.expected_output.estimated_remote_bytes = 1;
  tinyRatio.expected_output.estimated_output_bytes = 1;
  parseRequestManifest(tinyRatio);

  const nearSafe = clone(fixture("request-manifest-v1.json")) as Record<string, any>;
  nearSafe.source.expected_size_bytes = Number.MAX_SAFE_INTEGER;
  nearSafe.expected_output.full_product_bytes = Number.MAX_SAFE_INTEGER;
  nearSafe.expected_output.estimated_remote_bytes = Number.MAX_SAFE_INTEGER - 1;
  parseRequestManifest(nearSafe);

  const tooLargeInteger = clone(fixture("request-manifest-v1.json")) as Record<string, any>;
  tooLargeInteger.source.expected_size_bytes = Number.MAX_SAFE_INTEGER + 1;
  assert.throws(() => parseRequestManifest(tooLargeInteger), ManifestContractError);

  const oversized = clone(fixture("request-manifest-v1.json")) as Record<string, any>;
  const oversizedSwath = "s".repeat(128);
  oversized.request.swath = oversizedSwath;
  oversized.request.aoi_component_count = 64;
  oversized.mapping.windows = Array.from({length: 64}, (_, component_id) => ({
    ...clone(template),
    component_id,
    swath: oversizedSwath,
    dtype: "d".repeat(64),
  }));
  assert.throws(() => parseRequestManifest(oversized), /NISAR_SUBSET_MANIFEST_TOO_LARGE/);
  assert.throws(() => parseRequestManifestJson("{".repeat(32 * 1024 + 1)), /NISAR_SUBSET_MANIFEST_TOO_LARGE/);
});

test("transfer state matrix and RFC3339 validation fail closed", () => {
  const base = clone(fixture("transfer-manifest-v1.json")) as Record<string, any>;

  const planned = clone(base);
  planned.state = "PLANNED";
  planned.received_bytes = 0;
  for (const item of planned.ranges) {
    item.state = "PLANNED";
    item.received_bytes = 0;
    delete item.sha256;
  }
  parseTransferManifest(planned);

  const complete = clone(base);
  complete.state = "RANGES_COMPLETE";
  complete.received_bytes = complete.expected_remote_bytes;
  complete.ranges[1].state = "COMPLETE";
  complete.ranges[1].received_bytes = complete.ranges[1].expected_bytes;
  complete.ranges[1].sha256 = "4444444444444444444444444444444444444444444444444444444444444444";
  parseTransferManifest(complete);

  const failed = clone(base);
  failed.state = "FAILED";
  failed.ranges[1].state = "FAILED";
  failed.ranges[1].received_bytes = 0;
  delete failed.ranges[1].sha256;
  failed.received_bytes = failed.ranges[0].received_bytes;
  parseTransferManifest(failed);

  for (const state of ["INTERRUPTED", "SOURCE_CHANGED"] as const) {
    const recoverable = clone(base);
    recoverable.state = state;
    recoverable.ranges[1].state = "PLANNED";
    recoverable.ranges[1].received_bytes = 0;
    delete recoverable.ranges[1].sha256;
    recoverable.received_bytes = recoverable.ranges[0].received_bytes;
    parseTransferManifest(recoverable);
  }

  const invalidState = clone(planned);
  invalidState.state = "IN_PROGRESS";
  assert.throws(() => parseTransferManifest(invalidState), ManifestContractError);

  const invalidDate = clone(base);
  invalidDate.created_at = "2026-13-01T00:00:00Z";
  assert.throws(() => parseTransferManifest(invalidDate), ManifestContractError);

  const artifact = clone(fixture("artifact-manifest-v1.json")) as Record<string, any>;
  artifact.validity_masks[0].dataset_path = "\\science\\inputDataExceptionMask";
  assert.throws(() => parseArtifactManifest(artifact), ManifestContractError);
});

test("official Frequency B datasets and frequency-level QA masks match the contract", () => {
  const official = JSON.parse(readFileSync(
    resolve(root, "docs/fixtures/asf-nisar-subset-p0/official-single-scene-result.json"), "utf8",
  )) as any;
  for (const dataset of official.science.datasets as string[]) {
    const [frequencyGroup, polarization] = dataset.split("/");
    const frequency = frequencyGroup.replace("frequency", "");
    const value = clone(fixture("request-manifest-v1.json")) as any;
    Object.assign(value.request, {frequency, polarization});
    Object.assign(value.mapping.windows[0], {
      frequency, polarization, dataset_path: `/science/LSAR/RSLC/swaths/${dataset}`,
    });
    parseRequestManifest(value);
  }
  for (const polarization of ["HV", "VV"]) {
    const value = clone(fixture("request-manifest-v1.json")) as any;
    Object.assign(value.request, {frequency: "B", polarization});
    Object.assign(value.mapping.windows[0], {
      frequency: "B", polarization, dataset_path: `/science/LSAR/RSLC/swaths/frequencyB/${polarization}`,
    });
    parseRequestManifest(value);
  }
  const invalid = clone(fixture("request-manifest-v1.json")) as any;
  invalid.request.polarization = "VH";
  Object.assign(invalid.mapping.windows[0], {
    polarization: "VH", dataset_path: "/science/LSAR/RSLC/swaths/frequencyA/VH",
  });
  assert.throws(() => parseRequestManifest(invalid), ManifestContractError);
  const artifact = clone(fixture("artifact-manifest-v1.json")) as any;
  assert.equal(artifact.validity_masks[0].dataset_path, "/science/LSAR/RSLC/swaths/frequencyA/inputDataExceptionMask");
  artifact.validity_masks[0].dataset_path = "/science/LSAR/RSLC/swaths/frequencyA/HH/inputDataExceptionMask";
  assert.throws(() => parseArtifactManifest(artifact), ManifestContractError);
  assert.deepEqual(official.science.qa_masks, [
    "frequencyA/inputDataExceptionMask", "frequencyB/inputDataExceptionMask",
  ]);
});

function snapshotForRequest(request: any): any {
  const source = request.source;
  return {
    item_id: source.provider_product_key,
    provider_id: source.provider_id,
    metadata: {collection_profile_id: source.collection_profile_id},
    asset_access: {
      reference: {
        provider_id: source.provider_id,
        collection_id: source.collection_id,
        provider_product_id: source.provider_product_id,
        provider_product_key: source.provider_product_key,
        stable_asset_key: source.stable_asset_key,
      },
      role: "primary",
      file_name: source.file_name,
      expected_size_bytes: source.expected_size_bytes,
      checksum_algorithm: source.checksum?.algorithm.toUpperCase(),
      checksum_value: source.checksum?.value,
    },
    execution_policy: {collection_profile_id: source.collection_profile_id},
  };
}

test("Request binds to the frozen official NISAR primary HDF5 identity", () => {
  const request = parseRequestManifest(fixture("request-manifest-v1.json"));
  const snapshot = snapshotForRequest(request);
  validateRequestAgainstSnapshotItem(request, snapshot);
  const mutations: Array<(item: any) => void> = [
    (item) => { item.provider_id = "asf"; },
    (item) => { item.asset_access.reference.collection_id = "wrong"; },
    (item) => { item.asset_access.reference.provider_product_id = "wrong"; },
    (item) => { item.asset_access.reference.provider_product_key = "wrong"; },
    (item) => { item.asset_access.reference.stable_asset_key = "wrong"; },
    (item) => { item.asset_access.file_name = "wrong.h5"; },
    (item) => { item.asset_access.expected_size_bytes = 1; },
    (item) => { item.asset_access.checksum_value = "0".repeat(64); },
    (item) => { item.asset_access.role = "auxiliary"; },
    (item) => { item.execution_policy.collection_profile_id = "asf.sentinel1"; },
  ];
  for (const mutate of mutations) {
    const changed = clone(snapshot);
    mutate(changed);
    assert.throws(() => validateRequestAgainstSnapshotItem(request, changed), ManifestContractError);
  }
  const encoded = JSON.stringify(request).toLowerCase();
  assert.ok(!["http://", "https://", "credential_profile", "authorization", "cookie"].some((value) => encoded.includes(value)));
});

test("Artifact requires complete Transfer and exact output identity", async () => {
  const request = parseRequestManifest(fixture("request-manifest-v1.json"));
  const transferBase = clone(fixture("transfer-manifest-v1.json")) as any;
  const artifactBase = clone(fixture("artifact-manifest-v1.json")) as any;
  for (const state of ["IN_PROGRESS", "INTERRUPTED", "SOURCE_CHANGED", "FAILED"] as const) {
    const value = clone(transferBase);
    value.state = state;
    value.ranges[1].state = state === "FAILED" ? "FAILED" : "PLANNED";
    value.ranges[1].received_bytes = 0;
    delete value.ranges[1].sha256;
    value.received_bytes = value.ranges[0].received_bytes;
    const transfer = parseTransferManifest(value);
    const artifactValue = clone(artifactBase);
    artifactValue.transfer_manifest_sha256 = await canonicalSha256(transfer);
    const artifact = parseArtifactManifest(artifactValue);
    await assert.rejects(
      () => validateArtifactAgainstRequestAndTransfer(request, transfer, artifact),
      /NISAR_SUBSET_ARTIFACT_NOT_VALIDATED/,
    );
  }
  const transfer = parseTransferManifest(transferBase);
  for (const [key, value] of [["artifact_id", "different-subset"], ["artifact_file_name", "different_subset.h5"]]) {
    const changed = clone(artifactBase);
    changed[key] = value;
    await assert.rejects(
      () => validateArtifactAgainstRequestAndTransfer(request, transfer, parseArtifactManifest(changed)),
      /NISAR_SUBSET_IDENTITY_MISMATCH/,
    );
  }
});

test("separate finite sidecar budgets, component policy, optional masks, and UTC order are closed", async () => {
  const transferValue = clone(fixture("transfer-manifest-v1.json")) as any;
  transferValue.ranges = Array.from({length: NISAR_SUBSET_MAX_RANGES}, (_, index) => ({
    start: index, end_exclusive: index + 1, expected_bytes: 1, received_bytes: 1,
    sha256: index.toString(16).padStart(64, "0"), state: "COMPLETE",
  }));
  transferValue.expected_remote_bytes = NISAR_SUBSET_MAX_RANGES;
  transferValue.received_bytes = NISAR_SUBSET_MAX_RANGES;
  const largeTransfer = parseTransferManifest(transferValue);
  assert.ok(new TextEncoder().encode(canonicalJson(largeTransfer)).length <= NISAR_SUBSET_MAX_TRANSFER_CANONICAL_JSON_BYTES);
  transferValue.ranges.push(clone(transferValue.ranges.at(-1)));
  assert.throws(() => parseTransferManifest(transferValue), ManifestContractError);

  const artifactValue = clone(fixture("artifact-manifest-v1.json")) as any;
  artifactValue.dataset_windows = Array.from({length: 64}, (_, component_id) => ({
    ...clone(artifactValue.dataset_windows[0]), component_id,
  }));
  artifactValue.validity_masks = Array.from({length: 64}, (_, component_id) => ({
    ...clone(artifactValue.validity_masks[0]), component_id, sha256: component_id.toString(16).padStart(64, "0"),
  }));
  const largeArtifact = parseArtifactManifest(artifactValue);
  assert.ok(new TextEncoder().encode(canonicalJson(largeArtifact)).length <= NISAR_SUBSET_MAX_ARTIFACT_CANONICAL_JSON_BYTES);
  artifactValue.dataset_windows.push(clone(artifactValue.dataset_windows[0]));
  assert.throws(() => parseArtifactManifest(artifactValue), ManifestContractError);

  const missingComponent = clone(fixture("request-manifest-v1.json")) as any;
  missingComponent.request.aoi_component_count = 2;
  assert.throws(() => parseRequestManifest(missingComponent), ManifestContractError);
  const duplicate = clone(fixture("request-manifest-v1.json")) as any;
  duplicate.mapping.windows.push(clone(duplicate.mapping.windows[0]));
  assert.throws(() => parseRequestManifest(duplicate), ManifestContractError);

  const requestValue = clone(fixture("request-manifest-v1.json")) as any;
  requestValue.mapping.windows[0].validity_mask_required = false;
  const request = parseRequestManifest(requestValue);
  const transfer = clone(fixture("transfer-manifest-v1.json")) as any;
  transfer.request_manifest_sha256 = await canonicalSha256(request);
  const parsedTransfer = parseTransferManifest(transfer);
  const artifact = clone(fixture("artifact-manifest-v1.json")) as any;
  artifact.request_manifest_sha256 = await canonicalSha256(request);
  artifact.transfer_manifest_sha256 = await canonicalSha256(parsedTransfer);
  artifact.dataset_windows = request.mapping.windows;
  artifact.validity_masks = [];
  await validateArtifactAgainstRequestAndTransfer(request, parsedTransfer, parseArtifactManifest(artifact));

  const timeBase = clone(fixture("transfer-manifest-v1.json")) as any;
  for (const [created_at, updated_at] of [
    ["2026-08-01T00:00:00Z", "2026-08-01T00:00:00Z"],
    ["2026-08-01T00:00:00Z", "2026-08-01T00:00:01Z"],
    ["2026-08-01T01:00:00+01:00", "2026-08-01T00:00:00Z"],
  ]) parseTransferManifest({...clone(timeBase), created_at, updated_at});
  for (const [created_at, updated_at] of [
    ["2026-08-01T00:00:01Z", "2026-08-01T00:00:00Z"],
    ["2026-08-01T01:00:01+01:00", "2026-08-01T00:00:00Z"],
  ]) assert.throws(() => parseTransferManifest({...clone(timeBase), created_at, updated_at}), ManifestContractError);
});

test("cross-language canonical parity fixture executes all required edges", async () => {
  const parity = fixture("canonical-parity-v1.json") as any;
  for (const caseValue of parity.valid_cases) {
    const manifest = parseRequestManifest(buildParityCase(caseValue.builder));
    assert.equal(await canonicalSha256(manifest), caseValue.expected_sha256, caseValue.case);
  }
  for (const caseValue of parity.invalid_cases) {
    const parser = caseValue.target === "request" ? parseRequestManifest : parseArtifactManifest;
    const value = buildParityCase(caseValue.builder);
    assert.throws(() => parser(value), ManifestContractError, caseValue.case);
    if (caseValue.case === "request_over_32k") {
      assert.throws(
        () => embedRequestManifest({}, value as any),
        /NISAR_SUBSET_MANIFEST_TOO_LARGE/,
      );
    }
  }
});
