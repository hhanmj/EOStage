import assert from "node:assert/strict";
import test from "node:test";

import {
  MAX_PROJECTED_RENDER_LONGITUDE,
  MAX_UI_WORKBENCH_PAGE_SIZE,
  parseCoreAoiDisplay,
  parseCoreRestore,
  parseCoreTaskSummary,
  parseWorkbenchPage,
  projectCoreRestore,
  projectWorkbenchPage,
} from "./coreRestoreProjection.ts";
import { parseCanonicalGeometry, parseRenderGeometry } from "./geojsonGeometry.ts";

function taskSummaryPayload() {
  return {
    task_id: "task-telemetry", batch_id: "batch-telemetry", provider_id: "asf.sentinel1",
    created_at: "2026-08-19T00:00:00Z", status: "running", revision: 3,
    updated_at: "2026-08-19T00:00:03Z", selected_count: 1,
    delivery_requested_count: 2, delivery_rejected_count: 1, checked_count: 0,
    active_count: 1, paused_count: 0, queued_count: 0, completed_count: 0,
    failed_count: 0, skipped_count: 0, cancelled_count: 0, terminal_bytes: 0,
    active_bytes: 10, other_bytes: 0, transferred_bytes: 10, total_bytes: 100,
    bytes_per_second: 25, transfer_active_millis: 3_210,
    network_bytes_received: 16_900_000,
    source_total_bytes: 42_756_734_979,
    reusable_verified_bytes: 49_686_192,
    reused_verified_bytes: 47_876_062,
    active_item: { item_id: "ITEM-A", bytes_completed: 10, total_bytes: 100 },
    output_root: "E:\\downloads", layout: "direct", operation: null, actionable: true,
  };
}

test("Core task summary preserves Store-owned real transfer active time", () => {
  const parsed = parseCoreTaskSummary(taskSummaryPayload());
  assert.ok(parsed);
  assert.equal(parsed.transfer_active_millis, 3_210);
  assert.equal(parsed.delivery_requested_count, 2);
  assert.equal(parsed.delivery_rejected_count, 1);
  assert.equal(parsed.network_bytes_received, 16_900_000);
  assert.equal(parsed.source_total_bytes, 42_756_734_979);
  assert.equal(parsed.reusable_verified_bytes, 49_686_192);
  assert.equal(parsed.reused_verified_bytes, 47_876_062);
});

test("Core task summary keeps source total optional for old hosts and rejects unsafe values", () => {
  const current = taskSummaryPayload();
  const { source_total_bytes: _omitted, ...oldHost } = current;
  for (const payload of [oldHost, { ...current, source_total_bytes: null }]) {
    const parsed = parseCoreTaskSummary(payload);
    assert.ok(parsed);
    assert.equal(parsed.source_total_bytes, null);
  }
  for (const value of [-1, 1.5, Number.MAX_SAFE_INTEGER + 1, "10"]) {
    assert.equal(parseCoreTaskSummary({ ...current, source_total_bytes: value }), null);
  }
  assert.equal(parseCoreTaskSummary({ ...oldHost, source_total_byte: 1 }), null);
});

test("Core task summary parses nullable recovery facts with the strict byte boundary", () => {
  const unknown = parseCoreTaskSummary({
    ...taskSummaryPayload(),
    reusable_verified_bytes: null,
    reused_verified_bytes: null,
  });
  assert.ok(unknown);
  assert.equal(unknown.reusable_verified_bytes, null);
  assert.equal(unknown.reused_verified_bytes, null);

  for (const field of ["reusable_verified_bytes", "reused_verified_bytes"] as const) {
    for (const value of [-1, 1.5, Number.MAX_SAFE_INTEGER + 1, "10"]) {
      assert.equal(parseCoreTaskSummary({ ...taskSummaryPayload(), [field]: value }), null);
    }
  }
  assert.equal(parseCoreTaskSummary({ ...taskSummaryPayload(), reusable_verified_byte: 1 }), null);
});

test("Core task summary parses optional active progress groups and failure codes strictly", () => {
  const current = {
    ...taskSummaryPayload(),
    active_progress: { without_progress: 1, transferring: 0, idle: 2 },
    failure_codes: [
      { code: "NISAR_SUBSET_V2_AUTH_REFRESH_FAILED", local: true },
      { code: "NISAR_SUBSET_V2_BUDGET_UPGRADE_REFUSED", local: false },
    ],
  };
  const parsed = parseCoreTaskSummary(current);
  assert.ok(parsed);
  assert.deepEqual(parsed.active_progress, { without_progress: 1, transferring: 0, idle: 2 });
  assert.deepEqual(parsed.failure_codes, current.failure_codes);
  // L5 recheck 2026-10-19: a failure whose server certificate could not be verified.
  const certificate = parseCoreTaskSummary({
    ...taskSummaryPayload(),
    failure_codes: [{ code: "NISAR_SUBSET_V2_TRANSPORT_FAILED", local: false, tls_certificate: true }],
  });
  assert.ok(certificate);
  assert.deepEqual(certificate.failure_codes, [{ code: "NISAR_SUBSET_V2_TRANSPORT_FAILED", local: false, tls_certificate: true }]);
  // Downloader D1d: a failure whose diagnostic declares a network transport cause.
  const network = parseCoreTaskSummary({
    ...taskSummaryPayload(),
    failure_codes: [{ code: "DL005", local: false, network: true }, { code: "DL004", local: false, network: false }],
  });
  assert.ok(network);
  assert.deepEqual(network.failure_codes, [{ code: "DL005", local: false, network: true }, { code: "DL004", local: false, network: false }]);
  // Downloader D1: items waiting for a local resource (the NISAR output writer).
  const waiting = parseCoreTaskSummary({
    ...taskSummaryPayload(),
    active_progress: { without_progress: 0, transferring: 1, idle: 0, waiting: 2 },
  });
  assert.ok(waiting);
  assert.deepEqual(waiting.active_progress, { without_progress: 0, transferring: 1, idle: 0, waiting: 2 });
  // Older hosts omit both facts; they stay unknown rather than invented.
  const oldHost = parseCoreTaskSummary(taskSummaryPayload());
  assert.ok(oldHost);
  assert.equal(oldHost.active_progress, null);
  assert.equal(oldHost.failure_codes, null);
  for (const invalid of [
    { active_progress: { without_progress: -1, transferring: 0, idle: 0 } },
    { active_progress: { without_progress: 0, transferring: 0 } },
    { active_progress: { without_progress: 0, transferring: 0, idle: 0, writing: 1 } },
    { active_progress: { without_progress: 0, transferring: 0, idle: 0, waiting: -1 } },
    { active_progress: { without_progress: 0, transferring: 0, idle: 0, waiting: 1, writing: 1 } },
    { failure_codes: [{ code: "lower-case", local: false }] },
    { failure_codes: [{ code: "NISAR_X", local: "yes" }] },
    { failure_codes: [{ code: "NISAR_X", local: false, reason_zh: "provider text" }] },
    { failure_codes: [{ code: "NISAR_X", local: false, tls_certificate: "yes" }] },
    { failure_codes: [{ code: "DL005", local: false, network: "yes" }] },
    { failure_codes: Array.from({ length: 9 }, (_, index) => ({ code: `CODE_${index}`, local: false })) },
    { failure_codes: "NISAR_X" },
  ]) {
    assert.equal(parseCoreTaskSummary({ ...taskSummaryPayload(), ...invalid }), null, JSON.stringify(invalid));
  }
});

function corePayload() {
  return {
    readiness: { schema_version: 11, ready: true },
    workspace_context: null,
    region_aoi: null,
    region_aois: [],
    dem_source_identity: null as null | Record<string, unknown>,
    tasks: [],
    legacy_task_warnings: [{
      task_id: "legacy-task",
      provider_id: "legacy-provider",
      created_at: "2026-08-13T00:00:00Z",
      status: null,
      reason_code: "malformed_snapshot",
      compatibility_status: "unavailable",
    }],
    workbench: {
      batch_id: "batch-1",
      provider_id: "ASF",
      created_at: "2026-08-13T00:00:00Z",
      aoi_name: "AOI",
      query: { provider: "ASF", dataset: "SENTINEL-1", start: null, end: null, target_count: 10_000 },
      search_stats: {
        matched_count: 12_500,
        provider_returned_count: 10_000,
        frozen_candidate_count: 10_000,
        truncated: true,
        source_attempts: [] as Array<{
          provider_id: string;
          native_collection_id: string;
          outcome: "fallback" | "succeeded";
          fallback_reason: "transient_network" | "rate_limited" | "server_error" | null;
        }>,
        source: "ASF_BATCH",
      },
      candidate_count: 10_000,
      checked_count: 0,
      current_page_item_count: 100,
      page_index: 0,
      page_size: 100,
      revision: 4,
      deferred: true,
      reason_code: "candidate_payload_deferred",
    },
  };
}

function pagePayload(count = 1) {
  return {
    batch_id: "batch-1",
    revision: 4,
    page_index: 0,
    page_size: 100,
    total_count: 10_000,
    checked_count: 1,
    items: Array.from({ length: count }, (_, index) => ({
      ordinal: index,
      checked: index === 0,
      item: {
        item_id: `item-${index}`,
        provider_id: "ASF",
        display_name: `scene-${index}`,
        acquisition_datetime: null,
        footprint_geojson: index === 0
          ? { type: "Polygon", coordinates: [[[179, -45], [-179, -45], [-179, -35], [179, -35], [179, -45]]] }
          : null,
        metadata: {},
        payload: {},
        asset_access: null,
        execution_policy: null,
      },
    })),
  };
}

function aoiDisplayPayload() {
  return {
    region_id: "region-1",
    revision: 3,
    envelope: {
      south: -47,
      north: -34,
      crosses_antimeridian: true,
      display_west: 166,
      display_east: 185,
      request_windows: [
        { west: 166, south: -47, east: 180, north: -34, crs: "EPSG:4326" },
        { west: -180, south: -47, east: -175, north: -34, crs: "EPSG:4326" },
      ],
    },
  };
}

function nativeAoiDisplayPayload() {
  return {
    ...aoiDisplayPayload(),
    render_geometry: {
      type: "MultiPolygon",
      coordinates: [
        [[[166, -47], [179, -47], [179, -34], [166, -34], [166, -47]]],
        [[[181, -45], [185, -45], [185, -35], [181, -35], [181, -45]]],
      ],
    },
    render_bbox: { west: 166, south: -47, east: 185, north: -34 },
  };
}

function nativeRegionPayload() {
  return {
    region_id: "region-1",
    revision: 3,
    source_kind: "manual",
    geometry: {
      type: "MultiPolygon",
      coordinates: [
        [[[166, -47], [179, -47], [179, -34], [166, -34], [166, -47]]],
        [[[-179, -45], [-175, -45], [-175, -35], [-179, -35], [-179, -45]]],
      ],
    },
    bbox: [-179, -47, 179, -34],
    crs: "EPSG:4326",
    display_name: "Fixture AOI",
    bound_at: "2026-08-19T00:00:00Z",
    updated_at: "2026-08-19T00:00:00Z",
  };
}

function nativePageRenderPayload() {
  return {
    region_id: "region-1",
    revision: 3,
    items: [{
      item_id: "item-0",
      render_geometry: {
        type: "MultiPolygon",
        coordinates: [[[[179, -45], [181, -45], [181, -35], [179, -35], [179, -45]]]],
      },
      render_bbox: { west: 179, south: -45, east: 181, north: -35 },
    }],
  };
}

function pageRenderCopyAccepted(
  canonical: unknown,
  renderGeometry: unknown,
  renderBbox: { west: number; south: number; east: number; north: number },
  envelope: {
    south: number;
    north: number;
    crossesAntimeridian: boolean;
    displayWest: number;
    displayEast: number;
    requestWindows: readonly { west: number; south: number; east: number; north: number; crs: "EPSG:4326" }[];
  } | null = {
    south: 0,
    north: 1,
    crossesAntimeridian: false,
    displayWest: -180,
    displayEast: 180,
    requestWindows: [{ west: -180, south: 0, east: 180, north: 1, crs: "EPSG:4326" }],
  },
) {
  const page = pagePayload();
  page.items[0]!.item.footprint_geojson = canonical;
  const render = nativePageRenderPayload();
  render.items[0]!.render_geometry = renderGeometry;
  render.items[0]!.render_bbox = renderBbox;
  return projectWorkbenchPage({
    ok: true,
    readiness: { contract: "application.v1", status: "ready", store_schema_version: 11 },
    page,
    render,
  }, envelope ?? undefined).ok;
}

function largeCanonicalGeometry(componentCount = 277) {
  return {
    type: "MultiPolygon",
    coordinates: Array.from({ length: componentCount }, (_, index) => {
      const west = 166 + index * 0.05;
      const east = west + 0.02;
      const ring = Array.from({ length: 17 }, (_, point) => {
        const longitude = point % 3 === 0 ? west : point % 3 === 1 ? east : west + 0.01;
        const latitude = point < 8 ? -47 : -46.98;
        return [longitude, latitude];
      });
      ring.push(ring[0]!);
      return [ring];
    }),
  };
}

test("CoreRestore accepts the bounded native AOI display authority and rejects unsafe copies", () => {
  const authority = parseCoreAoiDisplay(nativeAoiDisplayPayload());
  assert.ok(authority);
  assert.equal(authority.region_id, "region-1");
  assert.equal(authority.revision, 3);
  assert.equal(authority.envelope.displayWest, 166);
  assert.equal(authority.envelope.displayEast, 185);
  assert.equal(authority.envelope.requestWindows.length, 2);

  const snapshot = projectCoreRestore({
    ok: true,
    readiness: { contract: "application.v1", status: "ready", store_schema_version: 11 },
    core: { ...corePayload(), region_aoi: nativeRegionPayload() },
    aoi_display: nativeAoiDisplayPayload(),
  });
  assert.equal(snapshot.ok, true);
  if (snapshot.ok) {
    assert.equal(snapshot.aoi_display?.envelope.displayEast, 185);
    assert.equal(snapshot.aoi_display?.render_bbox.east, 185);
  }

  const malformed = structuredClone(nativeAoiDisplayPayload());
  malformed.envelope.display_east = 270;
  assert.equal(parseCoreAoiDisplay(malformed), undefined);
  const mismatchedRenderBbox = structuredClone(nativeAoiDisplayPayload());
  mismatchedRenderBbox.render_bbox.west = 170;
  assert.equal(parseCoreAoiDisplay(mismatchedRenderBbox), undefined);
  assert.equal(parseCoreAoiDisplay({ ...nativeAoiDisplayPayload(), extra: true }), undefined);
});

test("canonical AOI GeoJSON keeps RFC bbox and foreign members while render DTO stays strict", () => {
  const canonical = structuredClone(nativeRegionPayload().geometry) as Record<string, unknown>;
  canonical.bbox = [166, -47, 179, -34];
  canonical["x-insar-owner"] = { source: "authoritative", flags: [true, false] };
  const ready = { contract: "application.v1", status: "ready", store_schema_version: 11 } as const;
  const result = projectCoreRestore({
    ok: true,
    readiness: ready,
    core: { ...corePayload(), region_aoi: { ...nativeRegionPayload(), geometry: canonical } },
    aoi_display: nativeAoiDisplayPayload(),
  });
  assert.equal(result.ok, true);
  if (result.ok) assert.deepEqual(result.core.region_aoi?.geometry, canonical);

  const renderExtra = structuredClone(nativeAoiDisplayPayload());
  renderExtra.render_geometry["x-render-extra"] = true;
  assert.equal(parseCoreAoiDisplay(renderExtra), undefined);
});

test("canonical GeoJSON admission rejects non-plain or non-JSON-safe foreign values", () => {
  const geometry = nativeRegionPayload().geometry as Record<string, unknown>;
  class GeometryRecord {
    type = geometry.type;
    coordinates = geometry.coordinates;
  }
  const classGeometry = new GeometryRecord();
  assert.equal(parseCoreAoiDisplay(nativeAoiDisplayPayload(), classGeometry), undefined);

  const unsafe = structuredClone(geometry);
  (unsafe as Record<string, unknown>)["x-unsafe"] = undefined;
  assert.equal(parseCoreAoiDisplay(nativeAoiDisplayPayload(), unsafe), undefined);
});

test("geometry snapshots reject 8 MiB padding plus hidden toJSON without invoking it", () => {
  const geometry = structuredClone(nativeRegionPayload().geometry) as Record<string, unknown>;
  geometry.padding = "x".repeat(8 * 1024 * 1024);
  let calls = 0;
  Object.defineProperty(geometry, "toJSON", {
    configurable: true,
    enumerable: false,
    get: () => {
      calls += 1;
      return () => geometry;
    },
  });
  assert.equal(parseCanonicalGeometry(geometry, 8 * 1024 * 1024), null);
  assert.equal(calls, 0);
});

test("render snapshots reject hidden extra keys without reading accessors", () => {
  const render = structuredClone(nativePageRenderPayload().items[0]!.render_geometry) as Record<string, unknown>;
  let calls = 0;
  Object.defineProperty(render, "hidden", {
    configurable: true,
    enumerable: false,
    get: () => {
      calls += 1;
      return true;
    },
  });
  assert.equal(parseRenderGeometry(render, 17 * 1024 * 1024, MAX_PROJECTED_RENDER_LONGITUDE), null);
  assert.equal(calls, 0);
});

test("geometry snapshots reject coordinate array accessors without invoking them", () => {
  const geometry = structuredClone(nativeRegionPayload().geometry) as Record<string, unknown>;
  const coordinates = geometry.coordinates as unknown[][][][];
  const position = coordinates[0]![0]![0]! as unknown[];
  let calls = 0;
  Object.defineProperty(position, "0", {
    configurable: true,
    enumerable: true,
    get: () => {
      calls += 1;
      return 166;
    },
  });
  assert.equal(parseCanonicalGeometry(geometry, 8 * 1024 * 1024), null);
  assert.equal(calls, 0);
});

test("canonical snapshots preserve legal enumerable toJSON data without invoking hooks", () => {
  for (const value of [null, "catalogue-member", { tag: "safe", rank: 1 }]) {
    const geometry = structuredClone(nativeRegionPayload().geometry) as Record<string, unknown>;
    geometry.toJSON = value;
    const parsed = parseCanonicalGeometry(geometry, 8 * 1024 * 1024);
    assert.ok(parsed);
    assert.deepEqual(parsed.toJSON, value);
  }

  let functionCalls = 0;
  const functionGeometry = structuredClone(nativeRegionPayload().geometry) as Record<string, unknown>;
  functionGeometry.toJSON = () => {
    functionCalls += 1;
    return functionGeometry;
  };
  assert.equal(parseCanonicalGeometry(functionGeometry, 8 * 1024 * 1024), null);
  assert.equal(functionCalls, 0);

  let accessorCalls = 0;
  const accessorGeometry = structuredClone(nativeRegionPayload().geometry) as Record<string, unknown>;
  Object.defineProperty(accessorGeometry, "toJSON", {
    configurable: true,
    enumerable: true,
    get: () => {
      accessorCalls += 1;
      return null;
    },
  });
  assert.equal(parseCanonicalGeometry(accessorGeometry, 8 * 1024 * 1024), null);
  assert.equal(accessorCalls, 0);

  const renderGeometry = structuredClone(nativeRegionPayload().geometry) as Record<string, unknown>;
  renderGeometry.toJSON = null;
  assert.equal(parseRenderGeometry(renderGeometry, 8 * 1024 * 1024, MAX_PROJECTED_RENDER_LONGITUDE), null);
});

test("canonical page footprints keep foreign members while page render DTO stays strict", () => {
  const canonical = structuredClone(pagePayload().items[0]!.item.footprint_geojson) as Record<string, unknown>;
  canonical.bbox = [179, -45, -179, -35];
  canonical["x-product-owner"] = { lineage: "catalogue", confidence: 1 };
  const page = pagePayload();
  page.items[0]!.item.footprint_geojson = canonical;
  const render = nativePageRenderPayload();
  assert.equal(projectWorkbenchPage({
    ok: true,
    readiness: { contract: "application.v1", status: "ready", store_schema_version: 11 },
    page,
    render,
  } as unknown as Parameters<typeof projectWorkbenchPage>[0]).ok, true);

  render.items[0]!.render_geometry["x-render-extra"] = true;
  assert.equal(projectWorkbenchPage({
    ok: true,
    readiness: { contract: "application.v1", status: "ready", store_schema_version: 11 },
    page,
    render,
  } as unknown as Parameters<typeof projectWorkbenchPage>[0]).ok, false);
});

test("native AOI authority rejects a render bbox from a shifted display envelope", () => {
  const shifted = nativeAoiDisplayPayload();
  shifted.envelope.display_west = 165;
  shifted.envelope.display_east = 184;
  shifted.envelope.request_windows = [
    { west: 165, south: -47, east: 180, north: -34, crs: "EPSG:4326" },
    { west: -180, south: -47, east: -176, north: -34, crs: "EPSG:4326" },
  ];
  assert.equal(parseCoreAoiDisplay(shifted, nativeRegionPayload().geometry), undefined);
});

test("native AOI authority rejects a conservative bbox whose latitude is broader than the envelope", () => {
  const broadened = nativeAoiDisplayPayload();
  broadened.render_bbox = { west: 166, south: -48, east: 185, north: -33 };
  assert.equal(parseCoreAoiDisplay(broadened, nativeRegionPayload().geometry), undefined);
});

test("CoreRestore rejects native AOI authority when identity is absent or stale", () => {
  const ready = { contract: "application.v1", status: "ready", store_schema_version: 11 } as const;
  const authority = nativeAoiDisplayPayload();
  const withoutRegion = projectCoreRestore({ ok: true, readiness: ready, core: corePayload(), aoi_display: authority });
  assert.equal(withoutRegion.ok, false);
  const stale = projectCoreRestore({
    ok: true,
    readiness: ready,
    core: { ...corePayload(), region_aoi: { ...nativeRegionPayload(), revision: 4 } },
    aoi_display: authority,
  });
  assert.equal(stale.ok, false);
  const differentRegion = projectCoreRestore({
    ok: true,
    readiness: ready,
    core: { ...corePayload(), region_aoi: { ...nativeRegionPayload(), region_id: "region-other" } },
    aoi_display: authority,
  });
  assert.equal(differentRegion.ok, false);
});

test("Workbench page accepts bounded native render copies for the current page", () => {
  const projected = projectWorkbenchPage({
    ok: true,
    readiness: { contract: "application.v1", status: "ready", store_schema_version: 11 },
    page: pagePayload(),
    render: nativePageRenderPayload(),
  } as unknown as Parameters<typeof projectWorkbenchPage>[0]);
  assert.equal(projected.ok, true);
  if (projected.ok) {
    assert.equal(projected.render?.region_id, "region-1");
    assert.equal(projected.render?.items[0]?.render_bbox.east, 181);
  }
});

test("native render geometry must stay a canonical world-copy of AOI and page footprints", () => {
  const ready = { contract: "application.v1", status: "ready", store_schema_version: 11 } as const;
  const wrongAoi = structuredClone(nativeAoiDisplayPayload());
  wrongAoi.render_geometry = {
    type: "Polygon",
    coordinates: [[[0, -1], [1, -1], [1, 0], [0, 0], [0, -1]]],
  };
  wrongAoi.render_bbox = { west: 0, south: -1, east: 1, north: 0 };
  assert.equal(projectCoreRestore({
    ok: true,
    readiness: ready,
    core: { ...corePayload(), region_aoi: nativeRegionPayload() },
    aoi_display: wrongAoi,
  }).ok, false);

  const oversized = structuredClone(nativeAoiDisplayPayload());
  oversized.render_geometry = {
    type: "MultiPolygon",
    coordinates: Array.from({ length: 65 }, () => [
      [[166, -47], [167, -47], [167, -46], [166, -46], [166, -47]],
    ]),
  };
  oversized.render_bbox = { west: 166, south: -47, east: 167, north: -34 };
  assert.equal(projectCoreRestore({
    ok: true,
    readiness: ready,
    core: { ...corePayload(), region_aoi: nativeRegionPayload() },
    aoi_display: oversized,
  }).ok, false);

  const wrongPageRender = structuredClone(nativePageRenderPayload());
  wrongPageRender.items[0]!.render_geometry = {
    type: "Polygon",
    coordinates: [[[0, -1], [1, -1], [1, 0], [0, 0], [0, -1]]],
  };
  wrongPageRender.items[0]!.render_bbox = { west: 0, south: -1, east: 1, north: 0 };
  assert.equal(projectWorkbenchPage({
    ok: true,
    readiness: ready,
    page: pagePayload(),
    render: wrongPageRender,
  } as unknown as Parameters<typeof projectWorkbenchPage>[0]).ok, false);

  const nullFootprintRender = structuredClone(nativePageRenderPayload());
  const nullFootprintPage = pagePayload();
  nullFootprintPage.items[0]!.item.footprint_geojson = null;
  assert.equal(projectWorkbenchPage({
    ok: true,
    readiness: ready,
    page: nullFootprintPage,
    render: nullFootprintRender,
  } as unknown as Parameters<typeof projectWorkbenchPage>[0]).ok, false);
});

test("native AOI authority accepts the producer's 8 MiB structural envelope beyond Provider caps", () => {
  const geometry = largeCanonicalGeometry();
  const lastWest = 166 + (277 - 1) * 0.05;
  const authority = {
    region_id: "region-large",
    revision: 9,
    envelope: {
      south: -47,
      north: -46.98,
      crosses_antimeridian: false,
      display_west: 166,
      display_east: lastWest + 0.02,
      request_windows: [{ west: 166, south: -47, east: lastWest + 0.02, north: -46.98, crs: "EPSG:4326" }],
    },
    render_geometry: geometry,
    render_bbox: { west: 166, south: -47, east: lastWest + 0.02, north: -46.98 },
  };
  const region = {
    region_id: "region-large",
    revision: 9,
    geometry,
  };
  const result = projectCoreRestore({
    ok: true,
    readiness: { contract: "application.v1", status: "ready", store_schema_version: 11 },
    core: { ...corePayload(), region_aoi: region },
    aoi_display: authority,
  });
  assert.equal(result.ok, true);
});

test("native global render authority accepts an 8.43 MiB-class output under the 17 MiB consumer ceiling", () => {
  const ring = Array.from({ length: 500_000 }, (_, index) => [
    index % 2 === 0 ? 170.01 : 170,
    -1 + (index % 3) * 0.001,
    1,
    2,
    3,
  ]);
  ring.push(ring[0]!);
  const renderGeometry = { type: "Polygon", coordinates: [ring] };
  const serialized = JSON.stringify(renderGeometry);
  assert.ok(serialized.length > 8 * 1024 * 1024);
  const parsed = parseCoreAoiDisplay({
    region_id: "region-large-output",
    revision: 1,
    envelope: {
      south: -1,
      north: -0.998,
      crosses_antimeridian: false,
      display_west: 170,
      display_east: 170.01,
      request_windows: [{ west: 170, south: -1, east: 170.01, north: -0.998, crs: "EPSG:4326" }],
    },
    render_geometry: renderGeometry,
    render_bbox: { west: 170, south: -1, east: 170.01, north: -0.998 },
  });
  assert.ok(parsed);
});

test("native page render budget counts UTF-8 bytes for non-ASCII item identities", () => {
  const itemId = "é".repeat(9_000_000);
  const page = pagePayload();
  page.items[0]!.item.item_id = itemId;
  const render = nativePageRenderPayload();
  render.items[0]!.item_id = itemId;
  render.items[0]!.render_geometry = null;
  render.items[0]!.render_bbox = null;
  assert.equal(parseWorkbenchPage({ ...page, render }), null);
});

test("native global render copies may be adjacent to the full-world display envelope", () => {
  const canonical = {
    type: "Polygon",
    coordinates: [[[170, -1], [-170, -1], [-170, 1], [170, 1], [170, -1]]],
  };
  const authority = {
    region_id: "region-global-adjacent",
    revision: 2,
    envelope: {
      south: -1,
      north: 1,
      crosses_antimeridian: true,
      display_west: 170,
      display_east: 190,
      request_windows: [
        { west: 170, south: -1, east: 180, north: 1, crs: "EPSG:4326" },
        { west: -180, south: -1, east: -170, north: 1, crs: "EPSG:4326" },
      ],
    },
    render_geometry: {
      type: "Polygon",
      coordinates: [[[170, -1], [190, -1], [190, 1], [170, 1], [170, -1]]],
    },
    render_bbox: { west: 170, south: -1, east: 190, north: 1 },
  };
  const result = projectCoreRestore({
    ok: true,
    readiness: { contract: "application.v1", status: "ready", store_schema_version: 11 },
    core: { ...corePayload(), region_aoi: { region_id: "region-global-adjacent", revision: 2, geometry: canonical } },
    aoi_display: authority,
  });
  assert.equal(result.ok, true);
});

test("native AOI authority rejects an envelope and bbox that only contain the render extrema", () => {
  const broadContainer = {
    region_id: "region-broad-container",
    revision: 1,
    envelope: {
      south: 29,
      north: 32,
      crosses_antimeridian: false,
      display_west: 99,
      display_east: 102,
      request_windows: [{ west: 99, south: 29, east: 102, north: 32, crs: "EPSG:4326" }],
    },
    render_geometry: {
      type: "Polygon",
      coordinates: [[[100, 29], [101, 29], [101, 32], [100, 32], [100, 29]]],
    },
    render_bbox: { west: 99, south: 29, east: 102, north: 32 },
  };
  assert.equal(parseCoreAoiDisplay(broadContainer), undefined);

  const fullWorld = {
    region_id: "region-full-world",
    revision: 2,
    envelope: {
      south: 0,
      north: 1,
      crosses_antimeridian: false,
      display_west: -180,
      display_east: 180,
      request_windows: [{ west: -180, south: 0, east: 180, north: 1, crs: "EPSG:4326" }],
    },
    render_geometry: {
      type: "Polygon",
      coordinates: [[[-180, 0], [180, 0], [180, 1], [-180, 1], [-180, 0]]],
    },
    render_bbox: { west: -180, south: 0, east: 180, north: 1 },
  };
  assert.ok(parseCoreAoiDisplay(fullWorld));

  const disconnectedPseudoFullWorld = structuredClone(fullWorld);
  disconnectedPseudoFullWorld.render_geometry = {
    type: "MultiPolygon",
    coordinates: [
      [[[-180, 0], [-179, 0], [-179, 1], [-180, 1], [-180, 0]]],
      [[[179, 0], [180, 0], [180, 1], [179, 1], [179, 0]]],
    ],
  };
  assert.equal(parseCoreAoiDisplay(disconnectedPseudoFullWorld), undefined);
});

test("native page copy matching assigns same-key near-tolerance components by geometry and world shift", () => {
  const canonical = {
    type: "MultiPolygon",
    coordinates: [
      [[[10, 0], [11, 0], [11, 1], [10, 1], [10, 0]]],
      [[[10.0000000000004, 0], [11.0000000000004, 0], [11.0000000000004, 1], [10.0000000000004, 1], [10.0000000000004, 0]]],
    ],
  };
  const shiftPolygon = (polygon: number[][][][], offset: number) => polygon.map((ring) =>
    ring.map((position) => [position[0]! + offset, ...position.slice(1)]),
  );
  const renderGeometry = {
    type: "MultiPolygon",
    coordinates: [
      canonical.coordinates[0]!,
      shiftPolygon(canonical.coordinates[0]!, 360),
      canonical.coordinates[1]!,
      shiftPolygon(canonical.coordinates[1]!, 360),
    ],
  };
  assert.equal(pageRenderCopyAccepted(
    canonical,
    renderGeometry,
    { west: 10, south: 0, east: 371.0000000000004, north: 1 },
    {
      south: 0,
      north: 1,
      crossesAntimeridian: false,
      displayWest: 10,
      displayEast: 371.0000000000004,
      requestWindows: [{ west: 10, south: 0, east: 371.0000000000004, north: 1, crs: "EPSG:4326" }],
    },
  ), true);

  const reorderedRenderGeometry = {
    type: "MultiPolygon",
    coordinates: [
      shiftPolygon(canonical.coordinates[1]!, 360),
      canonical.coordinates[0]!,
      canonical.coordinates[1]!,
      shiftPolygon(canonical.coordinates[0]!, 360),
    ],
  };
  assert.equal(pageRenderCopyAccepted(
    canonical,
    reorderedRenderGeometry,
    { west: 10, south: 0, east: 371.0000000000004, north: 1 },
    {
      south: 0,
      north: 1,
      crossesAntimeridian: false,
      displayWest: 10,
      displayEast: 371.0000000000004,
      requestWindows: [{ west: 10, south: 0, east: 371.0000000000004, north: 1, crs: "EPSG:4326" }],
    },
  ), true);

  const duplicateWorldGeometry = {
    type: "MultiPolygon",
    coordinates: [canonical.coordinates[0]!, canonical.coordinates[0]!, canonical.coordinates[1]!, canonical.coordinates[1]!],
  };
  assert.equal(pageRenderCopyAccepted(
    canonical,
    duplicateWorldGeometry,
    { west: 10, south: 0, east: 11.0000000000004, north: 1 },
    {
      south: 0,
      north: 1,
      crossesAntimeridian: false,
      displayWest: 10,
      displayEast: 11.0000000000004,
      requestWindows: [{ west: 10, south: 0, east: 11.0000000000004, north: 1, crs: "EPSG:4326" }],
    },
  ), false);
});

test("native page copy matching rejects a same-signature duplicate instead of rerouting to its neighbor", () => {
  const canonical = {
    type: "MultiPolygon",
    coordinates: [
      [[[10, 0], [11, 0], [11, 1], [10, 1], [10, 0]]],
      [[[10.0000000000001, 0], [11.0000000000001, 0], [11.0000000000001, 1], [10.0000000000001, 1], [10.0000000000001, 0]]],
    ],
  };
  const duplicate = {
    type: "MultiPolygon",
    coordinates: [canonical.coordinates[0]!, canonical.coordinates[0]!],
  };
  assert.equal(pageRenderCopyAccepted(
    canonical,
    duplicate,
    { west: 10, south: 0, east: 11, north: 1 },
    {
      south: 0,
      north: 1,
      crossesAntimeridian: false,
      displayWest: 10,
      displayEast: 11,
      requestWindows: [{ west: 10, south: 0, east: 11, north: 1, crs: "EPSG:4326" }],
    },
  ), false);
});

test("native page copy matching fails closed when canonical components have an identical expected key", () => {
  const rectangle = [[[10, 0], [11, 0], [11, 1], [10, 1], [10, 0]]];
  const canonical = { type: "MultiPolygon", coordinates: [rectangle, structuredClone(rectangle)] };
  const render = { type: "MultiPolygon", coordinates: [structuredClone(rectangle), structuredClone(rectangle)] };
  assert.equal(pageRenderCopyAccepted(
    canonical,
    render,
    { west: 10, south: 0, east: 11, north: 1 },
    null,
  ), false);
});

test("native AOI copy matching accepts producer-order and partial same-signature groups", () => {
  const canonical = {
    type: "MultiPolygon",
    coordinates: [
      [[[10, 0], [11, 0], [11, 1], [10, 1], [10, 0]]],
      [[[10.0000000000004, 0], [11.0000000000004, 0], [11.0000000000004, 1], [10.0000000000004, 1], [10.0000000000004, 0]]],
    ],
  };
  const shiftPolygon = (polygon: number[][][][], offset: number) => polygon.map((ring) =>
    ring.map((position) => [position[0]! + offset, ...position.slice(1)]),
  );
  const render = {
    type: "MultiPolygon",
    coordinates: [canonical.coordinates[0]!, canonical.coordinates[1]!],
  };
  const envelope = {
    south: 0,
    north: 1,
    crossesAntimeridian: false,
    displayWest: 10,
    displayEast: 11.0000000000004,
    requestWindows: [{ west: 10, south: 0, east: 11.0000000000004, north: 1, crs: "EPSG:4326" }],
  } as const;
  const authority = {
    region_id: "region-1",
    revision: 3,
    envelope: {
      south: envelope.south,
      north: envelope.north,
      crosses_antimeridian: envelope.crossesAntimeridian,
      display_west: envelope.displayWest,
      display_east: envelope.displayEast,
      request_windows: envelope.requestWindows,
    },
    render_geometry: render,
    render_bbox: { west: 10, south: 0, east: 11.0000000000004, north: 1 },
  };
  assert.ok(parseCoreAoiDisplay(authority, canonical));

  const partial = {
    type: "MultiPolygon",
    coordinates: [
      // Provider order is deliberately B+360, B0; numeric world-0 sorting
      // must still assign both copies to B in this partial page.
      shiftPolygon(canonical.coordinates[1]!, 360),
      canonical.coordinates[1]!,
    ],
  };
  assert.equal(pageRenderCopyAccepted(
    canonical,
    partial,
    { west: 10.0000000000004, south: 0, east: 371.0000000000004, north: 1 },
    {
      ...envelope,
      displayEast: 371.0000000000004,
      requestWindows: [{ west: 10, south: 0, east: 371.0000000000004, north: 1, crs: "EPSG:4326" }],
    },
  ), true);
});

test("native page copy matching stays bounded for 30k same-signature components", () => {
  const componentCount = 30_000;
  const signatureQuantum = Number.EPSILON * 180 * 8 * 2;
  const westCenter = Math.round(10 / signatureQuantum) * signatureQuantum;
  const west = westCenter - 50 * 3e-15;
  const east = westCenter + 1 + 49 * 3e-15;
  const south = -150 * 8e-16;
  const north = 1 + 149 * 8e-16;
  const canonicalCoordinates = Array.from({ length: componentCount }, (_, index) => {
    const componentWest = westCenter + (index % 100 - 50) * 3e-15;
    const componentSouth = (index / 100 | 0) * 8e-16 - 150 * 8e-16;
    return [[[componentWest, componentSouth], [componentWest + 1, componentSouth],
      [componentWest + 1, componentSouth + 1], [componentWest, componentSouth + 1],
      [componentWest, componentSouth]]];
  });
  const canonical = { type: "MultiPolygon", coordinates: canonicalCoordinates };
  const render = { type: "MultiPolygon", coordinates: canonicalCoordinates.map((polygon) => polygon) };
  assert.equal(pageRenderCopyAccepted(
    canonical,
    render,
    { west, south, east, north },
    {
      south,
      north,
      crossesAntimeridian: false,
      displayWest: west,
      displayEast: east,
      requestWindows: [{ west, south, east, north, crs: "EPSG:4326" }],
    },
  ), true);
});

test("native page copy matching requires exact producer copies across adjacent longitude buckets", () => {
  const west = 9.999999936051754;
  const canonical = {
    type: "Polygon",
    coordinates: [[[west, 0], [west + 1, 0], [west + 1, 1], [west, 1], [west, 0]]],
  };
  // A low-bit drift is not the Rust producer's raw-plus-world f64 result and
  // must fail closed under exact expected-copy matching.
  const shiftedWest = 369.999999936051836;
  const shiftedEast = 370.999999936051836;
  const shifted = {
    type: "Polygon",
    coordinates: [[
      [shiftedWest, 0], [shiftedEast, 0], [shiftedEast, 1], [shiftedWest, 1], [shiftedWest, 0],
    ]],
  };
  const envelope = {
    south: 0,
    north: 1,
    crossesAntimeridian: false,
    displayWest: shiftedWest,
    displayEast: shiftedEast,
    requestWindows: [{ west: shiftedWest, south: 0, east: shiftedEast, north: 1, crs: "EPSG:4326" }],
  } as const;
  assert.equal(pageRenderCopyAccepted(
    canonical,
    shifted,
    { west: shiftedWest, south: 0, east: shiftedEast, north: 1 },
    envelope,
  ), false);

  const exactShiftedWest = west + 360;
  const exactShiftedEast = west + 1 + 360;
  const exactShifted = {
    type: "Polygon",
    coordinates: [[
      [exactShiftedWest, 0], [exactShiftedEast, 0], [exactShiftedEast, 1], [exactShiftedWest, 1], [exactShiftedWest, 0],
    ]],
  };
  const exactEnvelope = {
    south: 0,
    north: 1,
    crossesAntimeridian: false,
    displayWest: exactShiftedWest,
    displayEast: exactShiftedEast,
    requestWindows: [{ west: exactShiftedWest, south: 0, east: exactShiftedEast, north: 1, crs: "EPSG:4326" }],
  } as const;
  assert.equal(pageRenderCopyAccepted(
    canonical,
    exactShifted,
    { west: exactShiftedWest, south: 0, east: exactShiftedEast, north: 1 },
    exactEnvelope,
  ), true);

  const differentStructure = {
    type: "Polygon",
    coordinates: [[
      [exactShiftedWest, 0], [exactShiftedWest + 0.5, 0], [exactShiftedEast, 0],
      [exactShiftedEast, 1], [exactShiftedWest, 1], [exactShiftedWest, 0],
    ]],
  };
  assert.equal(pageRenderCopyAccepted(
    canonical,
    differentStructure,
    { west: exactShiftedWest, south: 0, east: exactShiftedEast, north: 1 },
    exactEnvelope,
  ), false);
});

test("native page copy matching keeps three near-180 one-ULP rectangles distinct across world lifts", () => {
  const westValues = [-179.99999999999997, -179.99999999999994, -179.99999999999991];
  const rectangle = (west: number) => [[[west, 0], [west + 0.01, 0], [west + 0.01, 1], [west, 1], [west, 0]]];
  const canonical = { type: "MultiPolygon", coordinates: westValues.map(rectangle) };
  const shift = (polygon: number[][][], offset: number) => polygon.map((ring) =>
    ring.map((position) => [position[0]! + offset, ...position.slice(1)]),
  );
  const render = {
    type: "MultiPolygon",
    coordinates: [
      shift(canonical.coordinates[1]!, 360),
      shift(canonical.coordinates[0]!, 360),
      shift(canonical.coordinates[2]!, 360),
    ],
  };
  const renderPositions = render.coordinates.flat(2);
  const west = Math.min(...renderPositions.map((position) => position[0]!));
  const east = Math.max(...renderPositions.map((position) => position[0]!));
  const envelope = {
    south: 0,
    north: 1,
    crossesAntimeridian: false,
    displayWest: west,
    displayEast: east,
    requestWindows: [{ west, south: 0, east, north: 1, crs: "EPSG:4326" }],
  } as const;
  assert.equal(pageRenderCopyAccepted(canonical, render, { west, south: 0, east, north: 1 }, envelope), true);

  for (const [index, offset] of [360, 360, 360].entries()) {
    const singleCanonical = { type: "Polygon", coordinates: canonical.coordinates[index]! };
    const singleRender = { type: "Polygon", coordinates: offset === 0 ? canonical.coordinates[index]! : shift(canonical.coordinates[index]!, offset) };
    const singlePositions = singleRender.coordinates.flat();
    const singleWest = Math.min(...singlePositions.map((position) => position[0]!));
    const singleEast = Math.max(...singlePositions.map((position) => position[0]!));
    assert.equal(pageRenderCopyAccepted(
      singleCanonical,
      singleRender,
      { west: singleWest, south: 0, east: singleEast, north: 1 },
      {
        south: 0,
        north: 1,
        crossesAntimeridian: false,
        displayWest: singleWest,
        displayEast: singleEast,
        requestWindows: [{ west: singleWest, south: 0, east: singleEast, north: 1, crs: "EPSG:4326" }],
      },
    ), true);
  }
});

test("native page copy matching pairs a raw near-180 +360 copy with B0 and B+360", () => {
  const rectangle = (west: number, east: number) => [[[west, 0], [east, 0], [east, 1], [west, 1], [west, 0]]];
  const canonical = {
    type: "MultiPolygon",
    coordinates: [rectangle(179.9, 179.91), rectangle(179.9995, 179.99999999999997)],
  };
  const shift = (polygon: number[][][][], offset: number) => polygon.map((ring) =>
    ring.map((position) => [position[0]! + offset, ...position.slice(1)]),
  );
  const render = {
    type: "MultiPolygon",
    coordinates: [
      shift(canonical.coordinates[0]!, 360),
      canonical.coordinates[1]!,
      shift(canonical.coordinates[1]!, 360),
    ],
  };
  assert.equal(pageRenderCopyAccepted(
    canonical,
    render,
    { west: 179.9995, south: 0, east: 540, north: 1 },
    null,
  ), true);
});

test("native page copy matching rejects a third world copy for one canonical polygon", () => {
  const canonical = {
    type: "Polygon",
    coordinates: [[[10, 0], [11, 0], [11, 1], [10, 1], [10, 0]]],
  };
  const shift = (polygon: number[][][], offset: number) => polygon.map((ring) =>
    ring.map((position) => [position[0]! + offset, ...position.slice(1)]),
  );
  const render = {
    type: "MultiPolygon",
    coordinates: [
      shift(canonical.coordinates, -360),
      canonical.coordinates,
      shift(canonical.coordinates, 360),
    ],
  };
  assert.equal(pageRenderCopyAccepted(
    canonical,
    render,
    { west: -350, south: 0, east: 371, north: 1 },
    null,
  ), false);
});

test("native page render accepts the legal 541 seam edge and rejects beyond the producer bound", () => {
  assert.equal(MAX_PROJECTED_RENDER_LONGITUDE, 900);
  const canonical = {
    type: "Polygon",
    coordinates: [[[179, 0], [-179, 0], [-179, 1], [179, 1], [179, 0]]],
  };
  const legalRender = {
    type: "Polygon",
    coordinates: [[[539, 0], [541, 0], [541, 1], [539, 1], [539, 0]]],
  };
  assert.equal(pageRenderCopyAccepted(canonical, legalRender, { west: 539, south: 0, east: 541, north: 1 }, null), true);

  const outOfRange = {
    type: "Polygon",
    coordinates: [[[899, 0], [901, 0], [901, 1], [899, 1], [899, 0]]],
  };
  assert.equal(pageRenderCopyAccepted(canonical, outOfRange, { west: 899, south: 0, east: 901, north: 1 }, null), false);
});

test("native page copy matching covers component and reference shifts through k=2 but rejects k=3", () => {
  const canonical = {
    type: "Polygon",
    coordinates: [[[-179, 0], [179, 0], [179, 1], [-179, 1], [-179, 0]]],
  };
  const render = {
    type: "MultiPolygon",
    coordinates: [
      [[[181, 0], [179, 0], [179, 1], [181, 1], [181, 0]]],
      [[[541, 0], [539, 0], [539, 1], [541, 1], [541, 0]]],
    ],
  };
  const envelope = {
    south: 0,
    north: 1,
    crossesAntimeridian: false,
    displayWest: 179.5,
    displayEast: 539.25,
    requestWindows: [{ west: 179.5, south: 0, east: 539.25, north: 1, crs: "EPSG:4326" }],
  } as const;
  assert.equal(pageRenderCopyAccepted(
    canonical,
    render,
    { west: 179, south: 0, east: 541, north: 1 },
    envelope,
  ), true);

  const k3 = {
    type: "Polygon",
    coordinates: [[[901, 0], [899, 0], [899, 1], [901, 1], [901, 0]]],
  };
  assert.equal(pageRenderCopyAccepted(
    canonical,
    k3,
    { west: 899, south: 0, east: 901, north: 1 },
    null,
  ), false);
});

test("native page copy matching uses strict 2D parity while retaining canonical 3D identity", () => {
  const canonical = {
    type: "Polygon",
    coordinates: [[
      [179, 0, 10, 100], [-179, 0, 11, 101], [-179, 1, 12, 102], [179, 1, 13, 103], [179, 0, 10, 100],
    ]],
  };
  const render = {
    type: "Polygon",
    coordinates: [[
      [539, 0], [541, 0], [541, 1], [539, 1], [539, 0],
    ]],
  };
  const page = pagePayload();
  page.items[0]!.item.footprint_geojson = canonical;
  const nativeRender = nativePageRenderPayload();
  nativeRender.items[0]!.render_geometry = render;
  nativeRender.items[0]!.render_bbox = { west: 539, south: 0, east: 541, north: 1 };
  const projected = projectWorkbenchPage({
    ok: true,
    readiness: { contract: "application.v1", status: "ready", store_schema_version: 11 },
    page,
    render: nativeRender,
  } as unknown as Parameters<typeof projectWorkbenchPage>[0]);
  assert.equal(projected.ok, true);
  if (projected.ok) {
    assert.deepEqual(projected.page.items[0]!.item.footprint_geojson, canonical);
    assert.deepEqual(projected.render?.items[0]?.render_geometry, render);
  }
});

test("native page copy matching keeps two low-bit thin bands distinct under exact copies", () => {
  // One low bit at this magnitude remains distinguishable after the producer's
  // single `raw longitude + 360` operation at the render magnitude.
  const lowBit = Number.EPSILON * 256;
  const band = (west: number, east: number) => [[[west, 0], [east, 0], [east, 1], [west, 1], [west, 0]]];
  const canonical = {
    type: "MultiPolygon",
    coordinates: [
      band(10, 10 + lowBit),
      band(10 + lowBit * 2, 10 + lowBit * 3),
    ],
  };
  const exactCopy = (polygon: number[][][][]) => polygon.map((ring) =>
    ring.map((position) => [position[0]! + 360, position[1]!]),
  );
  const render = {
    type: "MultiPolygon",
    coordinates: canonical.coordinates.map(exactCopy),
  };
  const positions = render.coordinates.flat(2);
  const west = Math.min(...positions.map((position) => position[0]!));
  const east = Math.max(...positions.map((position) => position[0]!));
  assert.equal(pageRenderCopyAccepted(
    canonical,
    render,
    { west, south: 0, east, north: 1 },
    {
      south: 0,
      north: 1,
      crossesAntimeridian: false,
      displayWest: west,
      displayEast: east,
      requestWindows: [{ west, south: 0, east, north: 1, crs: "EPSG:4326" }],
    },
  ), true);
});

test("AOI and page render copies reject a uniform but irrelevant +360 lift", () => {
  const canonical = { type: "Polygon", coordinates: [[[100, 30], [101, 30], [101, 31], [100, 31], [100, 30]]] };
  const envelope = {
    south: 30,
    north: 31,
    crossesAntimeridian: false,
    displayWest: 100,
    displayEast: 101,
    requestWindows: [{ west: 100, south: 30, east: 101, north: 31, crs: "EPSG:4326" }],
  } as const;
  const irrelevant = {
    region_id: "region-irrelevant",
    revision: 1,
    envelope: {
      south: 30,
      north: 31,
      crosses_antimeridian: false,
      display_west: 100,
      display_east: 101,
      request_windows: [{ west: 100, south: 30, east: 101, north: 31, crs: "EPSG:4326" }],
    },
    render_geometry: { type: "Polygon", coordinates: [[[460, 30], [461, 30], [461, 31], [460, 31], [460, 30]]] },
    render_bbox: { west: 460, south: 30, east: 461, north: 31 },
  };
  assert.equal(parseCoreAoiDisplay(irrelevant, canonical), undefined);

  const page = pagePayload();
  page.items[0]!.item.footprint_geojson = canonical;
  const pageRender = nativePageRenderPayload();
  pageRender.items[0]!.render_geometry = {
    type: "Polygon",
    coordinates: [[[460, 30], [461, 30], [461, 31], [460, 31], [460, 30]]],
  };
  pageRender.items[0]!.render_bbox = { west: 460, south: 30, east: 461, north: 31 };
  assert.equal(projectWorkbenchPage({
    ok: true,
    readiness: { contract: "application.v1", status: "ready", store_schema_version: 11 },
    page,
    render: pageRender,
  } as unknown as Parameters<typeof projectWorkbenchPage>[0], envelope).ok, false);
});

test("native page render accepts a relevant component when another canonical component is outside the AOI", () => {
  const page = pagePayload();
  page.items[0]!.item.footprint_geojson = {
    type: "MultiPolygon",
    coordinates: [
      [[[170, 30], [171, 30], [171, 31], [170, 31], [170, 30]]],
      [[[100, 30], [101, 30], [101, 31], [100, 31], [100, 30]]],
    ],
  };
  const render = nativePageRenderPayload();
  render.items[0]!.render_geometry = {
    type: "Polygon",
    coordinates: [[[170, 30], [171, 30], [171, 31], [170, 31], [170, 30]]],
  };
  render.items[0]!.render_bbox = { west: 170, south: 30, east: 171, north: 31 };
  const envelope = {
    south: 30,
    north: 31,
    crossesAntimeridian: false,
    displayWest: 170,
    displayEast: 171,
    requestWindows: [{ west: 170, south: 30, east: 171, north: 31, crs: "EPSG:4326" }],
  } as const;
  assert.equal(projectWorkbenchPage({
    ok: true,
    readiness: { contract: "application.v1", status: "ready", store_schema_version: 11 },
    page,
    render,
  }, envelope).ok, true);
});

test("native render correspondence uses continuous tolerant lifts and rejects a 361-degree component", () => {
  const canonical = { type: "Polygon", coordinates: [[[0, 0], [1, 0], [1, 1], [0, 1], [0, 0]]] };
  const valid = {
    type: "Polygon",
    coordinates: [[[0, 0], [361.00000000000006, 0], [361.00000000000006, 1], [0, 1], [0, 0]]],
  };
  const validAuthority = {
    region_id: "region-lift",
    revision: 1,
    envelope: {
      south: 0,
      north: 1,
      crosses_antimeridian: false,
      display_west: 0,
      display_east: 1,
      request_windows: [{ west: 0, south: 0, east: 1, north: 1, crs: "EPSG:4326" }],
    },
    render_geometry: valid,
    render_bbox: { west: 0, south: 0, east: 361.00000000000006, north: 1 },
  };
  const validResult = projectCoreRestore({
    ok: true,
    readiness: { contract: "application.v1", status: "ready", store_schema_version: 11 },
    core: { ...corePayload(), region_aoi: { region_id: "region-lift", revision: 1, geometry: canonical } },
    aoi_display: validAuthority,
  });
  assert.equal(validResult.ok, false, "component width over 360 must fail even when point lifts are integral");

  const uniformAuthority = {
    ...validAuthority,
    envelope: {
      south: 0,
      north: 1,
      crosses_antimeridian: false,
      display_west: -180,
      display_east: 180,
      request_windows: [{ west: -180, south: 0, east: 180, north: 1, crs: "EPSG:4326" }],
    },
    render_geometry: {
      type: "Polygon",
      coordinates: [[[180.00000000000006, 0], [181.00000000000006, 0], [181.00000000000006, 1], [180.00000000000006, 1], [180.00000000000006, 0]]],
    },
    render_bbox: { west: 180.00000000000006, south: 0, east: 181.00000000000006, north: 1 },
  };
  const uniformCanonical = { type: "Polygon", coordinates: [[[-180, 0], [-179, 0], [-179, 1], [-180, 1], [-180, 0]]] };
  assert.equal(pageRenderCopyAccepted(
    uniformCanonical,
    uniformAuthority.render_geometry,
    uniformAuthority.render_bbox,
    {
      south: 0,
      north: 1,
      crossesAntimeridian: true,
      displayWest: -180,
      displayEast: 181,
      requestWindows: [
        { west: -180, south: 0, east: 180, north: 1, crs: "EPSG:4326" },
        { west: -180, south: 0, east: -179, north: 1, crs: "EPSG:4326" },
      ],
    },
  ), false);

  const exactUniformGeometry = {
    type: "Polygon",
    coordinates: [[[180, 0], [181, 0], [181, 1], [180, 1], [180, 0]]],
  };
  assert.equal(pageRenderCopyAccepted(
    uniformCanonical,
    exactUniformGeometry,
    { west: 180, south: 0, east: 181, north: 1 },
    {
      south: 0,
      north: 1,
      crossesAntimeridian: true,
      displayWest: -180,
      displayEast: 181,
      requestWindows: [
        { west: -180, south: 0, east: 180, north: 1, crs: "EPSG:4326" },
        { west: -180, south: 0, east: -179, north: 1, crs: "EPSG:4326" },
      ],
    },
  ), true);

  const crossingCanonical = {
    type: "Polygon",
    coordinates: [[[179.99999999999997, 0], [-179.99999999999997, 0], [-179.99999999999997, 1], [179.99999999999997, 1], [179.99999999999997, 0]]],
  };
  const crossing = {
    ...validAuthority,
    envelope: {
      south: 0,
      north: 1,
      crosses_antimeridian: false,
      display_west: -180,
      display_east: 180,
      request_windows: [{ west: -180, south: 0, east: 180, north: 1, crs: "EPSG:4326" }],
    },
    render_geometry: {
      type: "Polygon",
      coordinates: [[[179.99999999999997, 0], [180.00000000000003, 0], [180.00000000000003, 1], [179.99999999999997, 1], [179.99999999999997, 0]]],
    },
    render_bbox: { west: 179.99999999999997, south: 0, east: 180.00000000000003, north: 1 },
  };
  assert.equal(pageRenderCopyAccepted(crossingCanonical, crossing.render_geometry, crossing.render_bbox), true);
});

test("native full-world correspondence uses exact canonical endpoints and preserves edge direction", () => {
  const canonical = {
    type: "Polygon",
    coordinates: [[[-180, 0], [180, 0], [180, 1], [-180, 1], [-180, 0]]],
  };
  const fullWorldEnvelope = {
    south: 0,
    north: 1,
    crossesAntimeridian: false,
    displayWest: 180,
    displayEast: 540,
    requestWindows: [{ west: 180, south: 0, east: 540, north: 1, crs: "EPSG:4326" }],
  } as const;
  const adjacentCopy = {
    type: "Polygon",
    coordinates: [[[180, 0], [540, 0], [540, 1], [180, 1], [180, 0]]],
  };
  assert.equal(pageRenderCopyAccepted(
    canonical,
    adjacentCopy,
    { west: 180, south: 0, east: 540, north: 1 },
    fullWorldEnvelope,
  ), true);

  const reverseAdjacentCopy = {
    type: "Polygon",
    coordinates: [[[-180, 0], [-540, 0], [-540, 1], [-180, 1], [-180, 0]]],
  };
  assert.equal(pageRenderCopyAccepted(
    canonical,
    reverseAdjacentCopy,
    { west: -540, south: 0, east: -180, north: 1 },
    {
      ...fullWorldEnvelope,
      displayWest: -540,
      displayEast: -180,
      requestWindows: [{ west: -540, south: 0, east: -180, north: 1, crs: "EPSG:4326" }],
    },
  ), false);

  const nearGlobalCanonical = {
    type: "Polygon",
    coordinates: [[[-179.99999999999997, 0], [179.99999999999997, 0], [179.99999999999997, 1], [-179.99999999999997, 1], [-179.99999999999997, 0]]],
  };
  const fabricatedFullWorldCopy = {
    type: "Polygon",
    coordinates: [[[180.00000000000003, 0], [540, 0], [540, 1], [180.00000000000003, 1], [180.00000000000003, 0]]],
  };
  assert.equal(pageRenderCopyAccepted(
    nearGlobalCanonical,
    fabricatedFullWorldCopy,
    { west: 180.00000000000003, south: 0, east: 540, north: 1 },
    fullWorldEnvelope,
  ), false);
});

test("native render correspondence preserves the native directional tie at an exact 180-degree edge", () => {
  const canonical = {
    type: "Polygon",
    coordinates: [[[-90, 0], [90, 0], [90, 1], [-90, 1], [-90, 0]]],
  };
  const reversedTie = {
    region_id: "region-exact-tie",
    revision: 1,
    envelope: {
      south: 0,
      north: 1,
      crosses_antimeridian: false,
      display_west: -180,
      display_east: 180,
      request_windows: [{ west: -180, south: 0, east: 180, north: 1, crs: "EPSG:4326" }],
    },
    render_geometry: {
      type: "Polygon",
      coordinates: [[[-90, 0], [-270, 0], [-270, 1], [-90, 1], [-90, 0]]],
    },
    render_bbox: { west: -270, south: 0, east: -90, north: 1 },
  };
  assert.equal(pageRenderCopyAccepted(canonical, reversedTie.render_geometry, reversedTie.render_bbox), false);
});

test("native render correspondence does not apply the exact-180 tie rule to a near edge", () => {
  const canonical = {
    type: "Polygon",
    coordinates: [[[-90, 0], [90.000000005, 0], [90.000000005, 1], [-90, 1], [-90, 0]]],
  };
  const nearTieReverseLift = {
    region_id: "region-near-tie",
    revision: 1,
    envelope: {
      south: 0,
      north: 1,
      crosses_antimeridian: false,
      display_west: -180,
      display_east: 180,
      request_windows: [{ west: -180, south: 0, east: 180, north: 1, crs: "EPSG:4326" }],
    },
    render_geometry: {
      type: "Polygon",
      coordinates: [[[-90, 0], [-269.999999995, 0], [-269.999999995, 1], [-90, 1], [-90, 0]]],
    },
    render_bbox: { west: -269.999999995, south: 0, east: -90, north: 1 },
  };
  assert.equal(pageRenderCopyAccepted(canonical, nearTieReverseLift.render_geometry, nearTieReverseLift.render_bbox), true);
});

test("native render correspondence rejects a reverse lift beyond the few-ULP edge tolerance", () => {
  const canonical = {
    type: "Polygon",
    coordinates: [[[0, 0], [179.999999999999, 0], [179.999999999999, 1], [0, 1], [0, 0]]],
  };
  const authority = {
    region_id: "region-near-reverse",
    revision: 1,
    envelope: {
      south: 0,
      north: 1,
      crosses_antimeridian: false,
      display_west: -180,
      display_east: 180,
      request_windows: [{ west: -180, south: 0, east: 180, north: 1, crs: "EPSG:4326" }],
    },
    render_geometry: {
      type: "Polygon",
      coordinates: [[[0, 0], [-180.000000000001, 0], [-180.000000000001, 1], [0, 1], [0, 0]]],
    },
    render_bbox: { west: -180.000000000001, south: 0, east: 0, north: 1 },
  };
  assert.equal(pageRenderCopyAccepted(canonical, authority.render_geometry, authority.render_bbox), false);
});

test("page render authority rejects a boundary-only longitude alias", () => {
  const page = pagePayload();
  page.items[0]!.item.footprint_geojson = {
    type: "Polygon",
    coordinates: [[[1, 0], [2, 0], [2, 1], [1, 1], [1, 0]]],
  };
  const render = nativePageRenderPayload();
  render.items[0]!.render_geometry = {
    type: "Polygon",
    coordinates: [[[1, 0], [2, 0], [2, 1], [1, 1], [1, 0]]],
  };
  render.items[0]!.render_bbox = { west: 1, south: 0, east: 2, north: 1 };
  const envelope = {
    south: 0,
    north: 1,
    crosses_antimeridian: false,
    display_west: 0,
    display_east: 1,
    request_windows: [{ west: 0, south: 0, east: 1, north: 1, crs: "EPSG:4326" }],
  };
  assert.equal(projectWorkbenchPage({
    ok: true,
    readiness: { contract: "application.v1", status: "ready", store_schema_version: 11 },
    page,
    render,
  }, {
    south: 0,
    north: 1,
    crossesAntimeridian: false,
    displayWest: 0,
    displayEast: 1,
    requestWindows: [{ west: 0, south: 0, east: 1, north: 1, crs: "EPSG:4326" }],
  }).ok, false);
});

test("page render relevance requires every emitted component to overlap longitude but any component may overlap latitude", () => {
  const geometry = {
    type: "MultiPolygon",
    coordinates: [
      [[[0, 0], [1, 0], [1, 1], [0, 1], [0, 0]]],
      [[[0, 50], [1, 50], [1, 51], [0, 51], [0, 50]]],
    ],
  };
  const page = pagePayload();
  page.items[0]!.item.footprint_geojson = geometry;
  const render = nativePageRenderPayload();
  render.items[0]!.render_geometry = geometry;
  render.items[0]!.render_bbox = { west: 0, south: 0, east: 1, north: 51 };
  assert.equal(projectWorkbenchPage({
    ok: true,
    readiness: { contract: "application.v1", status: "ready", store_schema_version: 11 },
    page,
    render,
  }, {
    south: 0,
    north: 1,
    crossesAntimeridian: false,
    displayWest: 0,
    displayEast: 1,
    requestWindows: [{ west: 0, south: 0, east: 1, north: 1, crs: "EPSG:4326" }],
  }).ok, true);
});

test("CoreRestore projects legacy warnings as display-only and keeps candidates out", () => {
  const projected = parseCoreRestore(corePayload());
  assert.ok(projected);
  assert.equal(projected.workbench?.candidate_count, 10_000);
  assert.equal(projected.workbench?.query.target_count, 10_000);
  assert.equal(projected.workbench?.search_stats?.matched_count, 12_500);
  assert.equal(projected.legacy_task_warnings[0]?.actionable, false);
  assert.equal(projected.legacy_task_warnings[0]?.compatibility_status, "unavailable");

  const snapshot = projectCoreRestore({
    ok: true,
    readiness: { contract: "application.v1", status: "ready", store_schema_version: 11 },
    core: corePayload(),
  });
  assert.equal(snapshot.ok, true);
});

test("CoreRestore rejects a candidate array even when it is hidden in the bounded payload", () => {
  const payload = corePayload() as Record<string, unknown>;
  payload.candidates = [{ item_id: "must-not-cross" }];
  assert.equal(parseCoreRestore(payload), null);
});

test("CoreRestore accepts only the bounded canonical Store DEM identity", () => {
  const payload = corePayload();
  for (const updatedAt of [
    "2026-08-17T09:10:11Z",
    "2026-08-17T09:10:11.1Z",
    "2026-08-17T09:10:11.01Z",
    "2026-08-17T09:10:11.12Z",
    "2026-08-17T09:10:11.123Z",
  ]) {
    payload.dem_source_identity = {
      dataset: "COP90",
      provider: "OPENTOPOGRAPHY",
      updated_at: updatedAt,
    };
    assert.deepEqual(parseCoreRestore(payload)?.dem_source_identity, payload.dem_source_identity);
  }
  for (const invalid of [
    { ...payload.dem_source_identity, dataset: "cop90" },
    { ...payload.dem_source_identity, provider: " OPENTOPOGRAPHY" },
    { ...payload.dem_source_identity, updated_at: "2026-08-17T09:10:11.000Z" },
    { ...payload.dem_source_identity, updated_at: "2026-08-17T09:10:11.010Z" },
    { ...payload.dem_source_identity, updated_at: "2026-08-17T09:10:11.120Z" },
    { ...payload.dem_source_identity, updated_at: "2026-08-17T09:10:11.100Z" },
    { ...payload.dem_source_identity, snapshot_json: "{}" },
  ]) {
    const malformed = corePayload();
    malformed.dem_source_identity = invalid;
    assert.equal(parseCoreRestore(malformed), null);
  }
});

test("CoreRestore accepts fixed telemetry scalars but rejects unbounded task arrays", () => {
  const payload = corePayload();
  payload.tasks = [{
    task_id: "task-telemetry",
    batch_id: "batch-1",
    provider_id: "asf.sentinel1",
    created_at: "2026-08-15T00:00:00Z",
    status: "running",
    revision: 3,
    updated_at: "2026-08-15T00:00:03Z",
    selected_count: 3,
    checked_count: 1,
    active_count: 1,
    paused_count: 0,
    queued_count: 1,
    completed_count: 1,
    failed_count: 0,
    skipped_count: 0,
    cancelled_count: 0,
    terminal_bytes: 100,
    active_bytes: 55,
    other_bytes: 0,
    transferred_bytes: 155,
    total_bytes: null,
    bytes_per_second: 10,
    transfer_active_millis: 2_500,
    active_item: {
      item_id: "ITEM-B",
      bytes_completed: 25,
      total_bytes: 200,
    },
    output_root: "C:\\downloads",
    layout: "direct",
    operation: null,
    actionable: true,
  }];
  const task = parseCoreRestore(payload)?.tasks[0];
  assert.equal(task?.terminal_bytes, 100);
  assert.equal(task?.active_bytes, 55);
  assert.equal(task?.other_bytes, 0);
  assert.equal(task?.transferred_bytes, 155);
  assert.equal(task?.total_bytes, null);
  assert.equal(task?.transfer_active_millis, 2_500);
  assert.equal(task?.active_item?.item_id, "ITEM-B");

  for (const field of ["items", "candidates", "progress", "results"]) {
    const unsafe = structuredClone(payload) as { tasks: Array<Record<string, unknown>> };
    unsafe.tasks[0]![field] = [{ item_id: "must-not-cross" }];
    assert.equal(parseCoreRestore(unsafe), null, `${field} must not cross CoreRestore`);
  }
});

test("CoreRestore rejects unsafe page shape and malformed statistics while retaining distinct provider facts", () => {
  const unsafePage = corePayload();
  unsafePage.workbench.current_page_item_count = 101;
  assert.equal(parseCoreRestore(unsafePage), null);

  const zeroPageSize = corePayload();
  zeroPageSize.workbench.page_size = 0;
  zeroPageSize.workbench.current_page_item_count = 0;
  assert.equal(parseCoreRestore(zeroPageSize), null);

  const providerReturnedBelowFrozen = corePayload();
  providerReturnedBelowFrozen.workbench.search_stats.provider_returned_count = 9_999;
  assert.equal(
    parseCoreRestore(providerReturnedBelowFrozen)?.workbench?.search_stats?.provider_returned_count,
    9_999,
  );

  const matchedBelowReturned = corePayload();
  matchedBelowReturned.workbench.search_stats.matched_count = 9_999;
  assert.equal(
    parseCoreRestore(matchedBelowReturned)?.workbench?.search_stats?.matched_count,
    9_999,
  );

  const traced = corePayload();
  traced.workbench.provider_id = "element84.earth-search";
  traced.workbench.search_stats.source_attempts = [
    {
      provider_id: "microsoft.planetary-computer",
      native_collection_id: "landsat-c2-l2",
      outcome: "fallback",
      fallback_reason: "transient_network",
    },
    {
      provider_id: "element84.earth-search",
      native_collection_id: "landsat-c2-l2",
      outcome: "succeeded",
      fallback_reason: null,
    },
  ];
  assert.equal(
    parseCoreRestore(traced)?.workbench?.search_stats?.source_attempts.length,
    2,
  );

  const legacyStats = corePayload();
  const legacySearchStats = legacyStats.workbench.search_stats as unknown as Record<string, unknown>;
  delete legacySearchStats.source_attempts;
  assert.deepEqual(
    parseCoreRestore(legacyStats)?.workbench?.search_stats?.source_attempts,
    [],
  );

  const forgedTrace = structuredClone(traced);
  forgedTrace.workbench.search_stats.source_attempts[1].provider_id = "usgs.eros.m2m";
  assert.equal(parseCoreRestore(forgedTrace), null);

  const oversizedTrace = structuredClone(traced);
  oversizedTrace.workbench.search_stats.source_attempts = Array.from(
    { length: 9 },
    (_, index) => ({
      provider_id: index === 8 ? "element84.earth-search" : "microsoft.planetary-computer",
      native_collection_id: "landsat-c2-l2",
      outcome: index === 8 ? "succeeded" as const : "fallback" as const,
      fallback_reason: index === 8 ? null : "transient_network" as const,
    }),
  );
  assert.equal(parseCoreRestore(oversizedTrace), null);

  const malformedStats = corePayload() as unknown as { workbench: Record<string, unknown> };
  malformedStats.workbench.search_stats = {
    matched_count: 10_000,
    provider_returned_count: 10_000,
    frozen_candidate_count: 10_000,
    truncated: "yes",
    source_attempts: [],
    source: "ASF_BATCH",
  };
  assert.equal(parseCoreRestore(malformedStats), null);
});

test("Workbench page accepts the Store-native 250-object map projection and rejects over-sized transport", () => {
  const maximumPage = pagePayload(MAX_UI_WORKBENCH_PAGE_SIZE);
  maximumPage.page_size = MAX_UI_WORKBENCH_PAGE_SIZE;
  const oversizedPage = pagePayload(MAX_UI_WORKBENCH_PAGE_SIZE + 1);
  oversizedPage.page_size = MAX_UI_WORKBENCH_PAGE_SIZE;
  assert.equal(
    parseWorkbenchPage(maximumPage)?.items.length,
    MAX_UI_WORKBENCH_PAGE_SIZE,
  );
  assert.equal(parseWorkbenchPage(oversizedPage), null);

  const projected = projectWorkbenchPage({
    ok: true,
    readiness: { contract: "application.v1", status: "ready", store_schema_version: 11 },
    page: pagePayload(),
  });
  assert.equal(projected.ok, true);
});

test("Core task summary carries the downloader pre-scan progress and reuse counts", () => {
  const scan = { state: "scanning", scanned_items: 40, total_items: 250, reused_count: 31, unverified_count: 4 };
  const parsed = parseCoreTaskSummary({ ...taskSummaryPayload(), local_scan: scan });
  assert.ok(parsed);
  assert.deepEqual(parsed.local_scan, scan);
  assert.equal(parseCoreTaskSummary(taskSummaryPayload())?.local_scan, null);
  assert.equal(parseCoreTaskSummary({ ...taskSummaryPayload(), local_scan: null })?.local_scan, null);
  for (const malformed of [
    { ...scan, state: "hashing" },
    { ...scan, scanned_items: 251 },
    { ...scan, unverified_count: 32 },
    { ...scan, reused_count: -1 },
    { ...scan, path: "D:/out" },
  ]) {
    assert.equal(parseCoreTaskSummary({ ...taskSummaryPayload(), local_scan: malformed }), null);
  }
});

test("Core task summary accepts Sentinel-2 L1C multi-band batches and fails closed on tuple or bound mismatches", () => {
  const s2L1c = {
    ...taskSummaryPayload(),
    task_id: "task-s2-l1c",
    batch_id: "sentinel2-l1c-bands-batch-task-s2-l1c",
    provider_id: "copernicus.dataspace.odata",
    dataset_id: "optical.sentinel-2.l1c",
    selected_count: 13,
    delivery_requested_count: 1,
    delivery_rejected_count: 0,
    active_count: 13,
  };
  const parsed = parseCoreTaskSummary(s2L1c);
  assert.ok(parsed);
  assert.equal(parsed.selected_count, 13);
  assert.equal(parsed.delivery_requested_count, 1);
  assert.equal(parsed.delivery_rejected_count, 0);

  for (const invalid of [
    { provider_id: "usgs.eros.m2m" },
    { dataset_id: "optical.sentinel-2.l2a" },
    { batch_id: "sentinel2-bands-batch-task-s2-l1c" },
    { delivery_requested_count: 251 },
    { delivery_requested_count: 1, delivery_rejected_count: 1 },
    { delivery_requested_count: 3, delivery_rejected_count: 0, selected_count: 2 },
  ]) {
    assert.equal(parseCoreTaskSummary({ ...s2L1c, ...invalid }), null, JSON.stringify(invalid));
  }
});
