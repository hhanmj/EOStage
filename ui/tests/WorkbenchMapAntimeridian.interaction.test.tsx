import assert from "node:assert/strict";
import { registerHooks } from "node:module";
import test from "node:test";
import { JSDOM } from "jsdom";
import { createElement } from "react";
import { MAP_PREVIEW_PANE_Z_INDEX } from "../src/lib/mapLayerOrder.ts";

const dom = new JSDOM("<!doctype html><html><body></body></html>", { url: "http://127.0.0.1/" });
Object.assign(globalThis, {
  window: dom.window,
  document: dom.window.document,
  HTMLElement: dom.window.HTMLElement,
  Element: dom.window.Element,
  Node: dom.window.Node,
  MutationObserver: dom.window.MutationObserver,
  ResizeObserver: class { observe() {} disconnect() {} },
  requestAnimationFrame: (callback: FrameRequestCallback) => setTimeout(() => callback(Date.now()), 0),
  cancelAnimationFrame: (handle: number) => clearTimeout(handle),
  IS_REACT_ACT_ENVIRONMENT: true,
});
Object.defineProperty(globalThis, "navigator", { configurable: true, value: dom.window.navigator });
Object.assign(dom.window, {
  requestAnimationFrame: (callback: FrameRequestCallback) => setTimeout(() => callback(Date.now()), 0),
  cancelAnimationFrame: (handle: number) => clearTimeout(handle),
});

type RenderedMap = {
  container: HTMLElement;
  rerender: (ui: ReturnType<typeof createElement>) => void;
  unmount: () => void;
};
type BoundaryModule = typeof import("./reactLeafletBoundaryStub.tsx");

async function withProductionMap(
  suffix: string,
  props: Record<string, unknown>,
  check: (view: RenderedMap, boundary: BoundaryModule) => void | Promise<void>,
) {
  const hooks = registerHooks({
    resolve(specifier, context, nextResolve) {
      if (specifier === "react-leaflet" || specifier === "@/components/map/leafletBindings" || specifier.includes("leafletBindings")) {
        return { url: new URL("./reactLeafletBoundaryStub.tsx", import.meta.url).href, shortCircuit: true };
      }
      if (specifier === "leaflet") {
        return { url: new URL("./leafletBoundaryStub.ts", import.meta.url).href, shortCircuit: true };
      }
      return nextResolve(specifier, context);
    },
  });

  try {
    const boundary = await import("./reactLeafletBoundaryStub.tsx");
    boundary.resetBoundaryCapture();
    const { WorkbenchMap } = await import(`../src/components/WorkbenchMap.tsx?${suffix}`);
    const { render } = await import("@testing-library/react");
    const view = render(createElement(WorkbenchMap, props)) as RenderedMap;
    try {
      await check(view, boundary);
    } finally {
      view.unmount();
    }
  } finally {
    hooks.deregister();
  }
}

test("optical tile completion requires a successful image and rejects events from an old attempt", async () => {
  const suffix = "optical-tile-attempt-evidence";
  const changes: Array<{status: string; loadedCount: number}> = [];
  const props = {
    ...noopMapProps,
    preview: { product_key: "scene-optical", status: "loading", descriptor: null, url: null, message: "" },
    previewBbox: {west: 110, south: 30, east: 111, north: 31, crs: "EPSG:4326"},
    activePreviewProductKey: "scene-optical", showTileStatusOverlay: false,
    previewTileLayer: {productKey: "scene-optical", identity: "scene-optical:attempt-1",
      urlTemplate: "https://planetarycomputer.microsoft.com/fixture/{z}/{x}/{y}.png",
      attribution: "fixture", origin: "https://planetarycomputer.microsoft.com", statusLabel: "Fixture"},
    onPreviewTileStatusChange: (event: {status: string; loadedCount: number}) => changes.push(event),
  };
  await withProductionMap(suffix, props, async (view, boundary) => {
    const {act} = await import("@testing-library/react");
    const events = () => boundary.boundaryCapture().tileLayers.filter(row => row.url?.includes("/fixture/")).at(-1)!.eventHandlers;
    const old = events();
    await act(async () => { old.load(); });
    assert.equal(changes.at(-1)?.status, "loading", "Leaflet load alone is not image evidence");
    await act(async () => { old.tileerror(); old.load(); });
    assert.equal(changes.at(-1)?.status, "failed", "one failed tile without a success must not become ready");
    assert.equal(changes.at(-1)?.loadedCount, 0);
    const {WorkbenchMap} = await import(`../src/components/WorkbenchMap.tsx?${suffix}`);
    view.rerender(createElement(WorkbenchMap, {...props,
      previewTileLayer: {...props.previewTileLayer, identity: "scene-optical:attempt-2"},
    }));
    await act(async () => { old.tileload(); });
    assert.equal(changes.at(-1)?.status, "loading", "the first attempt cannot mark its replacement ready");
    assert.equal(changes.at(-1)?.loadedCount, 0);
    await act(async () => { events().tileload(); events().load(); });
    assert.equal(changes.at(-1)?.status, "ready");
    assert.equal(changes.at(-1)?.loadedCount, 1);
    assert.equal(view.container.querySelectorAll("[data-catalog-tile-preview-status]").length, 0);
  });
});

const nzAoiGeometry = {
  type: "MultiPolygon",
  coordinates: [
    [[[166, -47], [179, -47], [179, -34], [166, -34], [166, -47]]],
    [[[-179, -45], [-175, -45], [-175, -35], [-179, -35], [-179, -45]]],
  ],
};
const nzBbox = { west: -179, south: -47, east: 179, north: -34, crs: "EPSG:4326" };
const nzScene = {
  product_key: "scene-nz",
  scene_id: "scene-nz",
  platform: "S1A",
  product_type: "GRD",
  beam_mode: "IW",
  polarization: "VV",
  acquisition_datetime: "2026-08-17T00:00:00Z",
  absolute_orbit: null,
  relative_orbit: null,
  path: null,
  frame: null,
  orbit_direction: "ASCENDING",
  has_url: true,
  footprint_geojson: {
    type: "Polygon",
    coordinates: [[[179, -45], [-179, -45], [-179, -35], [179, -35], [179, -45]]],
  },
  footprint_bbox: { west: -179, south: -45, east: 179, north: -35, crs: "EPSG:4326" },
};
const nativeNzAoiGeometry = {
  type: "MultiPolygon",
  coordinates: [
    [[[166, -47], [179, -47], [179, -34], [166, -34], [166, -47]]],
    [[[181, -45], [185, -45], [185, -35], [181, -35], [181, -45]]],
  ],
};
const hubeiAoiGeometry = {
  type: "Polygon",
  coordinates: [[[108, 29], [116, 29], [116, 34], [108, 34], [108, 29]]],
};
const hubeiAuthority = {
  region_id: "region-hubei",
  revision: 3,
  envelope: {
    south: 29,
    north: 34,
    crossesAntimeridian: false,
    displayWest: 108,
    displayEast: 116,
    requestWindows: [{ west: 108, south: 29, east: 116, north: 34, crs: "EPSG:4326" }],
  },
  render_geometry: hubeiAoiGeometry,
  render_bbox: { west: 108, south: 29, east: 116, north: 34, crs: "EPSG:4326" },
};
const hubeiScene = {
  ...nzScene,
  product_key: "scene-hubei",
  scene_id: "scene-hubei",
  footprint_geojson: {
    type: "Polygon",
    coordinates: [[[110, 30], [112, 30], [112, 32], [110, 32], [110, 30]]],
  },
  footprint_bbox: { west: 110, south: 30, east: 112, north: 32, crs: "EPSG:4326" },
};

const noopMapProps = {
  layerKey: "osm",
  tiandituToken: "",
  drawMode: "rect",
  drawActive: false,
  onLayerChange: () => undefined,
  onSceneSelect: () => undefined,
  onDrawModeChange: () => undefined,
  onDrawActiveChange: () => undefined,
  onClearLayers: () => undefined,
  onRectDraw: () => undefined,
  onPolygonDraw: () => undefined,
  onPointDraw: () => undefined,
};

function nzMapProps(overrides: Record<string, unknown> = {}) {
  return {
    ...noopMapProps,
    bbox: nzBbox,
    aoiBbox: nzBbox,
    aoiGeometry: nzAoiGeometry,
    sceneBbox: null,
    scenes: [nzScene],
    selectedSceneId: null,
    preview: {
      status: "available",
      product_key: "scene-nz",
      descriptor: { render_profile_id: "asf.sentinel-1.grd-browse-geographic.v1" },
      url: "https://example.invalid/scene-nz.png",
      message: "",
    },
    activePreviewProductKey: "scene-nz",
    previewBbox: { west: -179, south: -45, east: -175, north: -35, crs: "EPSG:4326" },
    previewGeometry: nzScene.footprint_geojson,
    ...overrides,
  };
}

function longitudes(view: RenderedMap, selector: string): number[] {
  return Array.from(view.container.querySelectorAll(selector))
    .flatMap((node) => JSON.parse(node.getAttribute("data-positions") || "[]"))
    .flatMap((ring: unknown[]) => ring)
    .map((point: unknown) => Array.isArray(point) ? Number(point[1]) : Number.NaN);
}

test("production WorkbenchMap canonicalizes world-copy point drawing before AOI authority", async () => {
  let point: { lat: number; lng: number } | null = null;
  await withProductionMap("canonical-world-copy-point", nzMapProps({
    drawMode: "point",
    drawActive: true,
    onPointDraw: (lat: number, lng: number) => {
      point = { lat, lng };
    },
  }), async (_view, boundary) => {
    const { act } = await import("@testing-library/react");
    const events = boundary.boundaryCapture().mapEvents.find((item) => typeof item.click === "function");
    assert.ok(events, "DrawLayer map events were not registered");
    await act(async () => {
      events.click({ latlng: { lat: 30.1234, lng: 404.8077 } });
    });
    assert.deepEqual(point, { lat: 30.1234, lng: 44.8077 });
  });
});

test("production MapStatusOverlay never exposes Leaflet world-copy longitude", async () => {
  await withProductionMap("canonical-world-copy-status", nzMapProps(), async (view, boundary) => {
    const { act } = await import("@testing-library/react");
    const events = boundary.boundaryCapture().mapEvents.find(
      (item) => typeof item.mousemove === "function" && typeof item.moveend === "function",
    );
    assert.ok(events, "MapStatusOverlay map events were not registered");
    await act(async () => {
      events.mousemove({ latlng: { lat: 30.1234, lng: -315.1923 } });
      await new Promise<void>((resolve) => window.requestAnimationFrame(() => resolve()));
    });
    assert.match(view.container.textContent ?? "", /44\.8077, 30\.1234/);
    assert.doesNotMatch(view.container.textContent ?? "", /-315\.1923/);
  });
});

test("production Landsat tile Preview waits for real load and ignores stale tile events after product switch", async () => {
  const lifecycle: Array<{ productKey: string; status: string; loadedCount: number; errorCount: number }> = [];
  const first = {
    productKey: "landsat-first",
    identity: "landsat-first:mpc-landsat-true-color-v1",
    origin: "https://planetarycomputer.microsoft.com",
    urlTemplate: "https://planetarycomputer.microsoft.com/api/data/v1/item/tiles/WebMercatorQuad/{z}/{x}/{y}@1x?collection=landsat-c2-l2&item=LC08_L2SP_047027_20201204_02_T1&assets=red&assets=green&assets=blue&format=png",
    attribution: "Microsoft Planetary Computer · USGS Landsat",
  };
  const second = {
    ...first,
    productKey: "landsat-second",
    identity: "landsat-second:mpc-landsat-true-color-v1",
    urlTemplate: first.urlTemplate.replace(
      "LC08_L2SP_047027_20201204_02_T1",
      "LC09_L2SP_123045_20260101_02_T1",
    ),
  };
  const preview = (productKey: string) => ({
    status: "available",
    product_key: productKey,
    descriptor: { render_profile_id: "usgs.landsat.c2.l2.exact-scene-browse.v1" },
    url: "data:image/png;base64,AQID",
    message: "",
  });

  await withProductionMap("landsat-tile-preview-lifecycle", {
    ...noopMapProps,
    preview: preview(first.productKey),
    previewTileLayer: first,
    activePreviewProductKey: first.productKey,
    onPreviewTileStatusChange: (event: {
      productKey: string;
      status: string;
      loadedCount: number;
      errorCount: number;
    }) => lifecycle.push(event),
    previewBbox: { west: -123.5, south: 44.0, east: -122.0, north: 45.5 },
  }, async (view, boundary) => {
    const { act } = await import("@testing-library/react");
    const { WorkbenchMap } = await import("../src/components/WorkbenchMap.tsx?landsat-tile-preview-lifecycle");
    const status = () => view.container
      .querySelector("[data-catalog-tile-preview-status]")
      ?.getAttribute("data-catalog-tile-preview-status");

    assert.equal(status(), "loading");
    await act(async () => undefined);
    assert.equal(lifecycle.at(-1)?.productKey, first.productKey);
    assert.equal(lifecycle.at(-1)?.status, "loading");
    const firstLayer = boundary.boundaryCapture().tileLayers.find((layer) => layer.url === first.urlTemplate);
    assert.ok(firstLayer?.eventHandlers.load);
    assert.ok(firstLayer?.eventHandlers.tileload);
    assert.deepEqual(firstLayer?.bounds, [[44, -123.5], [45.5, -122]]);
    // Decision provenance (2026-11-16 Owner Request 10 Item 3): preview raster panes sit at
    // MAP_PREVIEW_PANE_Z_INDEX (620) above vector/footprint overlays so overlays are never occluded.
    assert.equal(
      view.container.querySelector('[data-pane-name="catalog-preview-tile-pane"]')?.getAttribute("data-pane-z-index"),
      String(MAP_PREVIEW_PANE_Z_INDEX),
    );
    assert.ok(firstLayer?.eventHandlers.tileerror);
    await act(async () => firstLayer.eventHandlers.tileload());
    assert.equal(status(), "ready");
    assert.equal(lifecycle.at(-1)?.status, "ready");
    assert.equal(lifecycle.at(-1)?.loadedCount, 1);

    view.rerender(createElement(WorkbenchMap, {
      ...noopMapProps,
      preview: preview(second.productKey),
      previewTileLayer: second,
      activePreviewProductKey: second.productKey,
      onPreviewTileStatusChange: (event: {
        productKey: string;
        status: string;
        loadedCount: number;
        errorCount: number;
      }) => lifecycle.push(event),
      previewBbox: { west: -123.5, south: 44.0, east: -122.0, north: 45.5 },
    }));
    await act(async () => undefined);
    assert.equal(status(), "loading");
    assert.equal(lifecycle.at(-1)?.productKey, second.productKey);
    assert.equal(lifecycle.at(-1)?.status, "loading");

    await act(async () => firstLayer.eventHandlers.tileerror());
    assert.equal(status(), "loading", "stale first-scene error must not fail the second scene");
    assert.equal(lifecycle.at(-1)?.productKey, second.productKey, "stale events must not reach the active callback");

    const secondLayer = boundary.boundaryCapture().tileLayers.find((layer) => layer.url === second.urlTemplate);
    assert.ok(secondLayer?.eventHandlers.tileerror);
    assert.ok(secondLayer?.eventHandlers.tileload);
    await act(async () => secondLayer.eventHandlers.tileerror());
    assert.equal(status(), "loading", "one out-of-footprint tile must not kill the whole Landsat layer");
    await act(async () => secondLayer.eventHandlers.tileerror());
    await act(async () => secondLayer.eventHandlers.tileerror());
    assert.equal(status(), "loading");
    await act(async () => secondLayer.eventHandlers.tileerror());
    assert.equal(status(), "failed");
    assert.equal(lifecycle.at(-1)?.productKey, second.productKey);
    assert.equal(lifecycle.at(-1)?.status, "failed");
    assert.equal(lifecycle.at(-1)?.errorCount, 4);
    assert.equal(
      view.container.querySelector(`[data-url="${second.urlTemplate}"]`),
      null,
      "failed preview tiles must unmount instead of retrying in a loop",
    );
  });
});

test("drawing the same polygon in the canonical and +360 worlds produces one canonical AOI", async () => {
  const draw = async (suffix: string, worldShift: number) => {
    let ring: [number, number][] | null = null;
    await withProductionMap(suffix, nzMapProps({
      drawMode: "polygon",
      drawActive: true,
      onPolygonDraw: (value: [number, number][]) => {
        ring = value;
      },
    }), async (_view, boundary) => {
      const { act } = await import("@testing-library/react");
      const point = (lng: number, lat: number) => ({
        lng,
        lat,
        distanceTo(other: { lng: number; lat: number }) {
          return Math.hypot(lng - other.lng, lat - other.lat) * 100_000;
        },
      });
      const points = [
        point(44 + worldShift, 30),
        point(45 + worldShift, 30),
        point(45 + worldShift, 31),
        point(44 + worldShift, 31),
      ];
      for (const value of points) {
        const events = boundary.boundaryCapture().mapEvents.filter((item) => typeof item.click === "function").at(-1);
        assert.ok(events);
        await act(async () => events.click({ latlng: value }));
      }
      const finish = boundary.boundaryCapture().mapEvents.filter((item) => typeof item.dblclick === "function").at(-1);
      assert.ok(finish);
      await act(async () => finish.dblclick({
        latlng: points.at(-1),
        originalEvent: { preventDefault() {} },
      }));
    });
    return ring;
  };

  const canonical = await draw("canonical-drawn-polygon", 0);
  const adjacentWorld = await draw("adjacent-world-drawn-polygon", 360);
  assert.deepEqual(adjacentWorld, canonical);
  assert.ok(adjacentWorld?.every(([longitude]) => longitude >= -180 && longitude <= 180));
});

test("a drawn dateline polygon is canonical while preserving two request windows", async () => {
  let ring: [number, number][] | null = null;
  await withProductionMap("drawn-dateline-polygon", nzMapProps({
    drawMode: "polygon",
    drawActive: true,
    onPolygonDraw: (value: [number, number][]) => {
      ring = value;
    },
  }), async (_view, boundary) => {
    const { act } = await import("@testing-library/react");
    const point = (lng: number, lat: number) => ({
      lng,
      lat,
      distanceTo(other: { lng: number; lat: number }) {
        return Math.hypot(lng - other.lng, lat - other.lat) * 100_000;
      },
    });
    const points = [point(170, 10), point(190, 10), point(190, 20), point(170, 20)];
    for (const value of points) {
      const events = boundary.boundaryCapture().mapEvents.filter((item) => typeof item.click === "function").at(-1);
      assert.ok(events);
      await act(async () => events.click({ latlng: value }));
    }
    const finish = boundary.boundaryCapture().mapEvents.filter((item) => typeof item.dblclick === "function").at(-1);
    assert.ok(finish);
    await act(async () => finish.dblclick({
      latlng: points.at(-1),
      originalEvent: { preventDefault() {} },
    }));
  });

  assert.ok(ring);
  assert.ok(ring.every(([longitude]) => longitude >= -180 && longitude <= 180));
  const { deriveWrappedAoiEnvelope } = await import("../src/lib/antimeridian.ts");
  const envelope = deriveWrappedAoiEnvelope({ type: "Polygon", coordinates: [ring] });
  assert.equal(envelope?.requestWindows.length, 2);
});

test("production WorkbenchMap renders the AOI layer in one New Zealand world copy", async () => {
  await withProductionMap("antimeridian-aoi", nzMapProps({ scenes: [], preview: undefined, previewGeometry: null }), async (view) => {
    const positions = longitudes(view, '[data-map-primitive="polygon"][data-class-name="aoi-overlay"]');
    assert.ok(positions.length > 0, "production AOI polygon was not rendered");
    assert.ok(positions.every((value) => value >= 160 && value <= 190));
    assert.match(view.container.textContent ?? "", /跨日期变更线.*2 段/);
  });
});

test("production WorkbenchMap renders scene footprint in the AOI world copy", async () => {
  await withProductionMap("antimeridian-scene", nzMapProps({ preview: undefined, previewGeometry: null }), async (view) => {
    const positions = longitudes(view, '[data-map-primitive="polygon"][data-class-name="scene-footprint"]');
    assert.ok(positions.length > 0, "production scene footprint was not rendered");
    assert.ok(positions.every((value) => value >= 160 && value <= 190));
  });
});

test("production WorkbenchMap keeps the New Zealand fit below 30 degrees", async () => {
  await withProductionMap("antimeridian-fit", nzMapProps({ scenes: [], preview: undefined, previewGeometry: null }), async (_view, boundary) => {
    const fit = boundary.boundaryCapture().fitBounds.at(-1) as [[number, number], [number, number]] | undefined;
    assert.ok(fit, "production FitToData did not capture bounds");
    assert.ok(fit[1][1] - fit[0][1] < 30, "New Zealand fit must remain local");
    assert.ok(fit[0][1] >= 160 && fit[1][1] <= 190);
  });
});

test("production WorkbenchMap keeps realistic decimal New Zealand AOI and scene layers visible in a local fit", async () => {
  const geometry = {
    type: "MultiPolygon",
    coordinates: [
      [[[165.5533714, -52.8213687], [179.3643594, -52.8213687], [179.3643594, -29.0303303], [165.5533714, -29.0303303], [165.5533714, -52.8213687]]],
      [[[-179.059153, -44.6349947], [-175.541422, -44.6349947], [-175.541422, -43.3108013], [-179.059153, -43.3108013], [-179.059153, -44.6349947]]],
    ],
  };
  const scene = {
    ...nzScene,
    footprint_geojson: {
      type: "Polygon",
      coordinates: [[[179.1, -45], [-179.1, -45], [-179.1, -35], [179.1, -35], [179.1, -45]]],
    },
    footprint_bbox: { west: -179.1, south: -45, east: 179.1, north: -35, crs: "EPSG:4326" },
  };
  await withProductionMap("antimeridian-realistic-decimals", nzMapProps({
    bbox: { west: -179.059153, south: -52.8213687, east: 179.3643594, north: -29.0303303, crs: "EPSG:4326" },
    aoiBbox: { west: -179.059153, south: -52.8213687, east: 179.3643594, north: -29.0303303, crs: "EPSG:4326" },
    aoiGeometry: geometry,
    scenes: [scene],
    preview: undefined,
    previewGeometry: null,
  }), async (view, boundary) => {
    assert.ok(view.container.querySelector('[data-map-primitive="polygon"][data-class-name="aoi-overlay"]'));
    assert.ok(view.container.querySelector('[data-map-primitive="polygon"][data-class-name="scene-footprint"]'));
    const fit = boundary.boundaryCapture().fitBounds.at(-1) as [[number, number], [number, number]] | undefined;
    assert.ok(fit);
    assert.ok(fit[1][1] - fit[0][1] < 30, "real provider decimals must not trigger a near-global fit");
  });
});

test("production WorkbenchMap gives an explicit Hubei focus bbox precedence over AOI", async () => {
  const hubei = { type: "Polygon", coordinates: [[[108, 30], [116, 30], [116, 40], [108, 40], [108, 30]]] };
  const focus = { west: 110, south: 32, east: 112, north: 34, crs: "EPSG:4326" };
  await withProductionMap("antimeridian-focus", nzMapProps({
    bbox: focus,
    aoiBbox: { west: 108, south: 30, east: 116, north: 40, crs: "EPSG:4326" },
    aoiGeometry: hubei,
    scenes: [],
    preview: undefined,
    previewGeometry: null,
  }), async (view, boundary) => {
    const fit = boundary.boundaryCapture().fitBounds.at(-1) as [[number, number], [number, number]] | undefined;
    assert.ok(fit, "production FitToData did not capture bounds");
    assert.deepEqual(fit, [[32, 110], [34, 112]]);
  });
});

test("production WorkbenchMap keeps ordinary China AOI, scene, Preview, and candidate bounds visible without page authority", async () => {
  const sceneBbox = { west: 110, south: 30, east: 112, north: 32, crs: "EPSG:4326" };
  await withProductionMap("china-noncrossing-fallback", nzMapProps({
    bbox: { west: 108, south: 29, east: 116, north: 34, crs: "EPSG:4326" },
    aoiBbox: { west: 108, south: 29, east: 116, north: 34, crs: "EPSG:4326" },
    aoiGeometry: hubeiAoiGeometry,
    aoiDisplayAuthority: hubeiAuthority,
    candidateBboxes: [sceneBbox],
    scenes: [hubeiScene],
    preview: {
      status: "available",
      product_key: "scene-hubei",
      descriptor: { render_profile_id: "asf.sentinel-1.grd-browse-geographic.v1" },
      url: "https://example.invalid/scene-hubei.png",
      message: "",
    },
    activePreviewProductKey: "scene-hubei",
    previewBbox: sceneBbox,
    previewGeometry: hubeiScene.footprint_geojson,
    previewRenderAuthority: undefined,
  }), async (view) => {
    assert.ok(view.container.querySelector('[data-map-primitive="polygon"][data-class-name="aoi-overlay"]'));
    assert.ok(view.container.querySelector('[data-map-primitive="polygon"][data-class-name="scene-footprint"]'));
    const previewBounds = view.container.querySelector('[data-map-primitive="image-overlay"]')?.getAttribute("data-bounds");
    assert.equal(previewBounds, JSON.stringify([[30, 110], [32, 112]]));
    const candidateBounds = view.container
      .querySelector('[data-map-primitive="rectangle"][data-class-name="admin-boundary-candidate"]')
      ?.getAttribute("data-bounds");
    assert.equal(candidateBounds, JSON.stringify([[30, 110], [32, 112]]));
  });
});

test("Gaode basemap projects China map copies without mutating WGS84 authority", async () => {
  const sceneBbox = { west: 110, south: 30, east: 112, north: 32, crs: "EPSG:4326" };
  const geometryBefore = structuredClone(hubeiAoiGeometry);
  const sceneBefore = structuredClone(hubeiScene);
  await withProductionMap("china-gcj-render-only", nzMapProps({
    layerKey: "gaodeSatellite",
    bbox: sceneBbox,
    aoiBbox: { west: 108, south: 29, east: 116, north: 34, crs: "EPSG:4326" },
    aoiGeometry: hubeiAoiGeometry,
    candidateBboxes: [sceneBbox],
    scenes: [hubeiScene],
    preview: undefined,
    previewGeometry: null,
  }), async (view, boundary) => {
    const aoiLongitudes = longitudes(view, '[data-map-primitive="polygon"][data-class-name="aoi-overlay"]');
    const sceneLongitudes = longitudes(view, '[data-map-primitive="polygon"][data-class-name="scene-footprint"]');
    assert.ok(Math.min(...aoiLongitudes) > 108);
    assert.ok(Math.min(...sceneLongitudes) > 110);
    const fit = boundary.boundaryCapture().fitBounds.at(-1) as [[number, number], [number, number]] | undefined;
    assert.ok(fit);
    assert.ok(fit[0][1] > sceneBbox.west);
    assert.ok(fit[1][1] > sceneBbox.east);
    assert.deepEqual(hubeiAoiGeometry, geometryBefore);
    assert.deepEqual(hubeiScene, sceneBefore);
  });
});

test("annotation preferences survive a labelled basemap and use the effective Tianditu token", async () => {
  const baseProps = nzMapProps({
    bbox: hubeiAuthority.render_bbox,
    aoiBbox: hubeiAuthority.render_bbox,
    aoiGeometry: hubeiAoiGeometry,
    scenes: [],
    preview: undefined,
    previewGeometry: null,
    tiandituToken: "owner-token",
  });
  await withProductionMap("annotation-preference", {
    ...baseProps,
    layerKey: "arcgisSatellite",
  }, async (view) => {
    const tileUrls = () => Array.from(view.container.querySelectorAll('[data-map-primitive="tile"]'))
      .map((node) => node.getAttribute("data-url") ?? "");
    assert.equal(tileUrls().length, 2);
    assert.ok(tileUrls().some((url) => url.includes("/cia_w/") && url.includes("tk=owner-token")));
    assert.ok(tileUrls().every((url) => !url.includes("/cva_w/")));

    view.rerender(createElement((await import("../src/components/WorkbenchMap.tsx")).WorkbenchMap, {
      ...baseProps,
      layerKey: "googleHybrid",
    }));
    assert.equal(tileUrls().length, 1);

    view.rerender(createElement((await import("../src/components/WorkbenchMap.tsx")).WorkbenchMap, {
      ...baseProps,
      layerKey: "googleSatellite",
    }));
    assert.equal(tileUrls().length, 2);

    view.rerender(createElement((await import("../src/components/WorkbenchMap.tsx")).WorkbenchMap, {
      ...baseProps,
      layerKey: "arcgisSatellite",
      tiandituToken: "",
    }));
    assert.equal(tileUrls().length, 2);
    assert.ok(tileUrls().some((url) => url.includes("/cia_w/") && !url.includes("tk=owner-token")));
    assert.doesNotMatch(view.container.textContent ?? "", /离线背景|切换到离线背景/);
  });
});

test("Beijing municipality and Hubei province AOIs admit the same valid scene footprints outside their envelopes", async () => {
  const cases = [
    {
      id: "beijing-municipality-footprint",
      aoi: { type: "Polygon", coordinates: [[[115.4, 39.4], [117.6, 39.4], [117.6, 41.1], [115.4, 41.1], [115.4, 39.4]]] },
      aoiBbox: { west: 115.4, south: 39.4, east: 117.6, north: 41.1, crs: "EPSG:4326" },
      scene: {
        ...hubeiScene,
        product_key: "scene-beijing",
        scene_id: "scene-beijing",
        footprint_geojson: { type: "Polygon", coordinates: [[[114, 38], [119, 38], [119, 43], [114, 43], [114, 38]]] },
        footprint_bbox: { west: 114, south: 38, east: 119, north: 43, crs: "EPSG:4326" },
      },
    },
    {
      id: "hubei-province-footprint",
      aoi: hubeiAoiGeometry,
      aoiBbox: { west: 108, south: 29, east: 116, north: 34, crs: "EPSG:4326" },
      scene: {
        ...hubeiScene,
        footprint_geojson: { type: "Polygon", coordinates: [[[106, 27], [118, 27], [118, 36], [106, 36], [106, 27]]] },
        footprint_bbox: { west: 106, south: 27, east: 118, north: 36, crs: "EPSG:4326" },
      },
    },
  ];

  for (const value of cases) {
    await withProductionMap(value.id, nzMapProps({
      bbox: value.aoiBbox,
      aoiBbox: value.aoiBbox,
      aoiGeometry: value.aoi,
      aoiDisplayAuthority: null,
      candidateBboxes: [],
      scenes: [value.scene],
      preview: undefined,
      previewGeometry: null,
    }), async (view) => {
      assert.ok(
        view.container.querySelector('[data-map-primitive="polygon"][data-class-name="scene-footprint"]'),
        `${value.id} must not suppress a valid SearchBatch footprint`,
      );
    });
  }
});

test("production WorkbenchMap renders the real AOI polygon and a distinct containing DEM envelope", async () => {
  const irregularHubei = {
    type: "Polygon",
    coordinates: [[[108, 31], [110, 29], [113, 29.5], [116, 32], [113, 34], [109, 33], [108, 31]]],
  };
  const aoiBbox = { west: 108, south: 29, east: 116, north: 34, crs: "EPSG:4326" };
  const demEnvelopeBbox = { west: 107.95, south: 28.95, east: 116.05, north: 34.05, crs: "EPSG:4326" };
  await withProductionMap("china-aoi-dem-envelope", nzMapProps({
    bbox: demEnvelopeBbox,
    aoiBbox,
    aoiGeometry: irregularHubei,
    aoiDisplayAuthority: null,
    demEnvelopeBbox,
    candidateBboxes: [],
    scenes: [],
    preview: undefined,
    previewGeometry: null,
  }), async (view) => {
    assert.ok(
      view.container.querySelector('[data-map-primitive="polygon"][data-class-name="aoi-overlay"]'),
      "real AOI polygon must remain visible",
    );
    assert.equal(
      view.container.querySelector('[data-map-primitive="rectangle"][data-class-name="aoi-overlay"]'),
      null,
      "AOI must not fall back to a bbox rectangle when polygon geometry exists",
    );
    const envelope = view.container.querySelector(
      '[data-map-primitive="rectangle"][data-class-name="dem-download-envelope"]',
    );
    assert.ok(envelope, "expanded DEM envelope must be rendered separately");
    assert.equal(envelope.getAttribute("data-bounds"), JSON.stringify([[28.95, 107.95], [34.05, 116.05]]));
    assert.match(view.container.textContent ?? "", /AOI 真实边界/);
    assert.match(view.container.textContent ?? "", /DEM 下载范围/);
  });
});

test("production WorkbenchMap projects a bbox-only Preview into the same world copy", async () => {
  await withProductionMap("antimeridian-preview-bbox", nzMapProps({
    scenes: [],
    previewGeometry: null,
    previewBbox: { west: -179, south: -45, east: -175, north: -35, crs: "EPSG:4326" },
  }), async (view) => {
    const previewBounds = view.container.querySelector('[data-map-primitive="image-overlay"]')?.getAttribute("data-bounds");
    assert.ok(previewBounds, "bbox-only Preview must remain visible");
    const bounds = JSON.parse(previewBounds!) as [[number, number], [number, number]];
    assert.deepEqual(bounds, [[-45, 181], [-35, 185]]);
  });
});

test("production WorkbenchMap fail-closes a matching native Preview with explicit null render authority", async () => {
  const authority = {
    region_id: "region-nz",
    revision: 2,
    envelope: {
      south: -47,
      north: -34,
      crossesAntimeridian: true,
      displayWest: 166,
      displayEast: 185,
      requestWindows: [
        { west: 166, south: -47, east: 180, north: -34, crs: "EPSG:4326" },
        { west: -180, south: -47, east: -175, north: -34, crs: "EPSG:4326" },
      ],
    },
    render_geometry: nativeNzAoiGeometry,
    render_bbox: { west: 166, south: -47, east: 185, north: -34 },
  };
  await withProductionMap("antimeridian-preview-native-null", nzMapProps({
    aoiDisplayAuthority: authority,
    previewRenderAuthority: {
      region_id: "region-nz",
      revision: 2,
      render_geometry: null,
      render_bbox: null,
    },
  }), async (view) => {
    assert.equal(view.container.querySelector('[data-map-primitive="image-overlay"]'), null);
  });
});

test("production WorkbenchMap hides a Preview with no page authority while native AOI authority is active", async () => {
  const authority = {
    region_id: "region-nz",
    revision: 2,
    envelope: {
      south: -47,
      north: -34,
      crossesAntimeridian: true,
      displayWest: 166,
      displayEast: 185,
      requestWindows: [
        { west: 166, south: -47, east: 180, north: -34, crs: "EPSG:4326" },
        { west: -180, south: -47, east: -175, north: -34, crs: "EPSG:4326" },
      ],
    },
    render_geometry: nativeNzAoiGeometry,
    render_bbox: { west: 166, south: -47, east: 185, north: -34 },
  };
  await withProductionMap("antimeridian-preview-native-missing", nzMapProps({
    aoiDisplayAuthority: authority,
    previewRenderAuthority: undefined,
  }), async (view) => {
    assert.equal(view.container.querySelector('[data-map-primitive="image-overlay"]'), null);
  });
});

test("production WorkbenchMap still hides a scene with no page authority for a crossing AOI", async () => {
  const authority = {
    region_id: "region-nz",
    revision: 2,
    envelope: {
      south: -47,
      north: -34,
      crossesAntimeridian: true,
      displayWest: 166,
      displayEast: 185,
      requestWindows: [
        { west: 166, south: -47, east: 180, north: -34, crs: "EPSG:4326" },
        { west: -180, south: -47, east: -175, north: -34, crs: "EPSG:4326" },
      ],
    },
    render_geometry: nativeNzAoiGeometry,
    render_bbox: { west: 166, south: -47, east: 185, north: -34 },
  };
  await withProductionMap("antimeridian-scene-native-missing", nzMapProps({
    aoiDisplayAuthority: authority,
    scenes: [nzScene],
    preview: undefined,
    previewGeometry: null,
  }), async (view) => {
    assert.equal(view.container.querySelector('[data-map-primitive="polygon"][data-class-name="scene-footprint"]'), null);
  });
});

test("production WorkbenchMap hides a stale native Preview authority instead of falling back to canonical coordinates", async () => {
  const authority = {
    region_id: "region-nz",
    revision: 2,
    envelope: {
      south: -47,
      north: -34,
      crossesAntimeridian: true,
      displayWest: 166,
      displayEast: 185,
      requestWindows: [
        { west: 166, south: -47, east: 180, north: -34, crs: "EPSG:4326" },
        { west: -180, south: -47, east: -175, north: -34, crs: "EPSG:4326" },
      ],
    },
    render_geometry: nativeNzAoiGeometry,
    render_bbox: { west: 166, south: -47, east: 185, north: -34 },
  };
  await withProductionMap("antimeridian-preview-native-mismatch", nzMapProps({
    aoiDisplayAuthority: authority,
    previewRenderAuthority: {
      region_id: "region-old",
      revision: 1,
      render_geometry: nativeNzAoiGeometry,
      render_bbox: { west: 166, south: -47, east: 185, north: -34 },
    },
  }), async (view) => {
    assert.equal(view.container.querySelector('[data-map-primitive="image-overlay"]'), null);
  });
});

test("production WorkbenchMap consumes native exact-180 authority without rotating outside its copy", async () => {
  const exact180 = {
    type: "Polygon",
    coordinates: [[[-90, 0], [90, 0], [90, 1], [-90, 1], [-90, 0]]],
  };
  const authority = {
    region_id: "region-exact-180",
    revision: 1,
    envelope: {
      south: 0,
      north: 1,
      crossesAntimeridian: false,
      displayWest: -90,
      displayEast: 90,
      requestWindows: [{ west: -90, south: 0, east: 90, north: 1, crs: "EPSG:4326" }],
    },
    render_geometry: exact180,
    render_bbox: { west: -90, south: 0, east: 90, north: 1 },
  };
  await withProductionMap("antimeridian-native-exact-180", nzMapProps({
    bbox: { west: -90, south: 0, east: 90, north: 1, crs: "EPSG:4326" },
    aoiBbox: { west: -90, south: 0, east: 90, north: 1, crs: "EPSG:4326" },
    aoiGeometry: exact180,
    aoiDisplayAuthority: authority,
    scenes: [],
    preview: undefined,
    previewGeometry: null,
  }), async (view, boundary) => {
    const positions = longitudes(view, '[data-map-primitive="polygon"][data-class-name="aoi-overlay"]');
    assert.ok(positions.every((value) => value >= -90 && value <= 90));
    const fit = boundary.boundaryCapture().fitBounds.at(-1) as [[number, number], [number, number]] | undefined;
    assert.deepEqual(fit, [[0, -90], [1, 90]]);
  });
});

test("production WorkbenchMap uses the native scene copy even when it extends beyond the AOI envelope", async () => {
  const authority = {
    region_id: "region-nz",
    revision: 2,
    envelope: {
      south: -47,
      north: -34,
      crossesAntimeridian: true,
      displayWest: 166,
      displayEast: 185,
      requestWindows: [
        { west: 166, south: -47, east: 180, north: -34, crs: "EPSG:4326" },
        { west: -180, south: -47, east: -175, north: -34, crs: "EPSG:4326" },
      ],
    },
    render_geometry: nativeNzAoiGeometry,
    render_bbox: { west: 166, south: -47, east: 185, north: -34 },
  };
  const seamCanonical = {
    type: "Polygon",
    coordinates: [[[179, -45], [-159, -45], [-159, -35], [179, -35], [179, -45]]],
  };
  const nativeScene = {
    ...nzScene,
    footprint_geojson: seamCanonical,
    footprint_bbox: { west: -159, south: -45, east: 179, north: -35, crs: "EPSG:4326" },
    render_geometry: {
      type: "Polygon",
      coordinates: [[[179, -45], [201, -45], [201, -35], [179, -35], [179, -45]]],
    },
    render_bbox: { west: 179, south: -45, east: 201, north: -35 },
    render_authority: { region_id: "region-nz", revision: 2 },
  };
  await withProductionMap("antimeridian-native-scene-extends-aoi", nzMapProps({
    aoiDisplayAuthority: authority,
    scenes: [nativeScene],
    preview: undefined,
    previewGeometry: null,
  }), async (view) => {
    const positions = longitudes(view, '[data-map-primitive="polygon"][data-class-name="scene-footprint"]');
    assert.ok(positions.includes(201));
    assert.ok(positions.every((value) => value >= 179 && value <= 201));
  });
});

test("production WorkbenchMap renders every disconnected component from native AOI authority", async () => {
  const disconnected = {
    type: "MultiPolygon",
    coordinates: [
      [[[100, 0], [101, 0], [101, 1], [100, 1], [100, 0]]],
      [[[120, 0], [121, 0], [121, 1], [120, 1], [120, 0]]],
    ],
  };
  const authority = {
    region_id: "region-disconnected",
    revision: 1,
    envelope: {
      south: 0,
      north: 1,
      crossesAntimeridian: false,
      displayWest: 100,
      displayEast: 121,
      requestWindows: [{ west: 100, south: 0, east: 121, north: 1, crs: "EPSG:4326" }],
    },
    render_geometry: disconnected,
    render_bbox: { west: 100, south: 0, east: 121, north: 1 },
  };
  await withProductionMap("antimeridian-native-disconnected", nzMapProps({
    bbox: { west: 100, south: 0, east: 121, north: 1, crs: "EPSG:4326" },
    aoiBbox: { west: 100, south: 0, east: 121, north: 1, crs: "EPSG:4326" },
    aoiGeometry: disconnected,
    aoiDisplayAuthority: authority,
    scenes: [],
    preview: undefined,
    previewGeometry: null,
  }), async (view) => {
    const polygons = view.container.querySelectorAll('[data-map-primitive="polygon"][data-class-name="aoi-overlay"]');
    assert.equal(polygons.length, 2);
    const positions = longitudes(view, '[data-map-primitive="polygon"][data-class-name="aoi-overlay"]');
    assert.ok(positions.includes(100));
    assert.ok(positions.includes(120));
  });
});

test("production WorkbenchMap hides unsupported exact-180 AOI without native render authority", async () => {
  const exact180 = {
    type: "Polygon",
    coordinates: [[[-90, 0], [90, 0], [90, 1], [-90, 1], [-90, 0]]],
  };
  await withProductionMap("antimeridian-unsupported-aoi-hide", nzMapProps({
    bbox: { west: -90, south: 0, east: 90, north: 1, crs: "EPSG:4326" },
    aoiBbox: { west: -90, south: 0, east: 90, north: 1, crs: "EPSG:4326" },
    aoiGeometry: exact180,
    scenes: [],
    preview: undefined,
    previewGeometry: null,
  }), async (view) => {
    assert.equal(view.container.querySelector('[data-map-primitive="polygon"][data-class-name="aoi-overlay"]'), null);
  });
});

test("production WorkbenchMap renders native global AOI copies as two visible polygons", async () => {
  const seamAoi = {
    type: "Polygon",
    coordinates: [[[179, -1], [-179, -1], [-179, 1], [179, 1], [179, -1]]],
  };
  const authority = {
    region_id: "region-global",
    revision: 4,
    envelope: {
      south: -1,
      north: 1,
      crossesAntimeridian: false,
      displayWest: -180,
      displayEast: 180,
      requestWindows: [{ west: -180, south: -1, east: 180, north: 1, crs: "EPSG:4326" }],
    },
    render_geometry: {
      type: "MultiPolygon",
      coordinates: [
        [[[179, -1], [180, -1], [180, 1], [179, 1], [179, -1]]],
        [[[-180, -1], [-179, -1], [-179, 1], [-180, 1], [-180, -1]]],
      ],
    },
    render_bbox: { west: -180, south: -1, east: 180, north: 1 },
  };
  await withProductionMap("antimeridian-native-global-copies", nzMapProps({
    bbox: { west: -180, south: -1, east: 180, north: 1, crs: "EPSG:4326" },
    aoiBbox: { west: -180, south: -1, east: 180, north: 1, crs: "EPSG:4326" },
    aoiGeometry: seamAoi,
    aoiDisplayAuthority: authority,
    scenes: [],
    preview: undefined,
    previewGeometry: null,
  }), async (view) => {
    const polygons = view.container.querySelectorAll('[data-map-primitive="polygon"][data-class-name="aoi-overlay"]');
    assert.equal(polygons.length, 2);
    const positions = longitudes(view, '[data-map-primitive="polygon"][data-class-name="aoi-overlay"]');
    assert.ok(positions.includes(-179));
    assert.ok(positions.includes(179));
  });
});

test("production WorkbenchMap renders a full-world scene copy and hides its wide Preview", async () => {
  const fullWorld = {
    type: "Polygon",
    coordinates: [[[-180, 0], [180, 0], [180, 1], [-180, 1], [-180, 0]]],
  };
  const fullWorldRender = {
    type: "Polygon",
    coordinates: [[[180, 0], [540, 0], [540, 1], [180, 1], [180, 0]]],
  };
  const authority = {
    region_id: "region-full-world-scene",
    revision: 1,
    envelope: {
      south: 0,
      north: 1,
      crossesAntimeridian: false,
      displayWest: -180,
      displayEast: 180,
      requestWindows: [{ west: -180, south: 0, east: 180, north: 1, crs: "EPSG:4326" }],
    },
    render_geometry: fullWorld,
    render_bbox: { west: -180, south: 0, east: 180, north: 1 },
  };
  const fullWorldScene = {
    ...nzScene,
    product_key: "scene-full-world",
    scene_id: "scene-full-world",
    footprint_geojson: fullWorld,
    footprint_bbox: { west: -180, south: 0, east: 180, north: 1, crs: "EPSG:4326" },
    render_geometry: fullWorldRender,
    render_bbox: { west: 180, south: 0, east: 540, north: 1, crs: "EPSG:4326" },
    render_authority: { region_id: "region-full-world-scene", revision: 1 },
  };
  await withProductionMap("antimeridian-native-full-world-scene", nzMapProps({
    bbox: { west: -180, south: 0, east: 180, north: 1, crs: "EPSG:4326" },
    aoiBbox: { west: -180, south: 0, east: 180, north: 1, crs: "EPSG:4326" },
    aoiGeometry: fullWorld,
    aoiDisplayAuthority: authority,
    scenes: [fullWorldScene],
    preview: undefined,
    previewGeometry: null,
    previewRenderAuthority: {
      region_id: "region-full-world-scene",
      revision: 1,
      render_geometry: fullWorldRender,
      render_bbox: { west: 180, south: 0, east: 540, north: 1, crs: "EPSG:4326" },
    },
  }), async (view) => {
    const positions = longitudes(view, '[data-map-primitive="polygon"][data-class-name="scene-footprint"]');
    assert.ok(positions.includes(540));
    assert.equal(view.container.querySelector('[data-map-primitive="image-overlay"]'), null);
  });
});

test("production WorkbenchMap renders both exact seam scene copies on a full-world AOI and hides the wide Preview", async () => {
  const fullWorld = {
    type: "Polygon",
    coordinates: [[[-180, 0], [180, 0], [180, 1], [-180, 1], [-180, 0]]],
  };
  const fullWorldAuthority = {
    region_id: "region-full-world-seam-scene",
    revision: 1,
    envelope: {
      south: 0,
      north: 1,
      crossesAntimeridian: false,
      displayWest: -180,
      displayEast: 180,
      requestWindows: [{ west: -180, south: 0, east: 180, north: 1, crs: "EPSG:4326" }],
    },
    render_geometry: fullWorld,
    render_bbox: { west: -180, south: 0, east: 180, north: 1 },
  };
  const seamCanonical = {
    type: "Polygon",
    coordinates: [[[170, -1], [-170, -1], [-170, 1], [170, 1], [170, -1]]],
  };
  const seamRendered = {
    type: "MultiPolygon",
    coordinates: [
      [[[-190, -1], [-170, -1], [-170, 1], [-190, 1], [-190, -1]]],
      [[[170, -1], [190, -1], [190, 1], [170, 1], [170, -1]]],
    ],
  };
  const seamScene = {
    ...nzScene,
    product_key: "scene-full-world-seam",
    scene_id: "scene-full-world-seam",
    footprint_geojson: seamCanonical,
    footprint_bbox: { west: -170, south: -1, east: 170, north: 1, crs: "EPSG:4326" },
    render_geometry: seamRendered,
    render_bbox: { west: -190, south: -1, east: 190, north: 1, crs: "EPSG:4326" },
    render_authority: { region_id: "region-full-world-seam-scene", revision: 1 },
  };
  await withProductionMap("antimeridian-native-full-world-seam-scene", nzMapProps({
    bbox: { west: -180, south: 0, east: 180, north: 1, crs: "EPSG:4326" },
    aoiBbox: { west: -180, south: 0, east: 180, north: 1, crs: "EPSG:4326" },
    aoiGeometry: fullWorld,
    aoiDisplayAuthority: fullWorldAuthority,
    scenes: [seamScene],
    preview: {
      status: "available",
      product_key: "scene-full-world-seam",
      descriptor: { render_profile_id: "asf.sentinel-1.grd-browse-geographic.v1" },
      url: "https://example.invalid/scene-full-world-seam.png",
      message: "",
    },
    activePreviewProductKey: "scene-full-world-seam",
    previewGeometry: seamCanonical,
    previewRenderAuthority: {
      region_id: "region-full-world-seam-scene",
      revision: 1,
      render_geometry: seamRendered,
      render_bbox: { west: -190, south: -1, east: 190, north: 1, crs: "EPSG:4326" },
    },
  }), async (view) => {
    const polygons = view.container.querySelectorAll('[data-map-primitive="polygon"][data-class-name="scene-footprint"]');
    assert.equal(polygons.length, 2);
    const positions = longitudes(view, '[data-map-primitive="polygon"][data-class-name="scene-footprint"]');
    assert.ok(positions.includes(-190));
    assert.ok(positions.includes(190));
    assert.equal(view.container.querySelector('[data-map-primitive="image-overlay"]'), null);
  });
});

test("production WorkbenchMap rejects a reversed full-world scene copy", async () => {
  const fullWorld = {
    type: "Polygon",
    coordinates: [[[-180, 0], [180, 0], [180, 1], [-180, 1], [-180, 0]]],
  };
  const reversedRender = {
    type: "Polygon",
    coordinates: [[[540, 0], [180, 0], [180, 1], [540, 1], [540, 0]]],
  };
  const authority = {
    region_id: "region-full-world-reverse",
    revision: 1,
    envelope: {
      south: 0,
      north: 1,
      crossesAntimeridian: false,
      displayWest: -180,
      displayEast: 180,
      requestWindows: [{ west: -180, south: 0, east: 180, north: 1, crs: "EPSG:4326" }],
    },
    render_geometry: fullWorld,
    render_bbox: { west: -180, south: 0, east: 180, north: 1 },
  };
  const scene = {
    ...nzScene,
    product_key: "scene-full-world-reverse",
    scene_id: "scene-full-world-reverse",
    footprint_geojson: fullWorld,
    footprint_bbox: { west: -180, south: 0, east: 180, north: 1, crs: "EPSG:4326" },
    render_geometry: reversedRender,
    render_bbox: { west: 180, south: 0, east: 540, north: 1, crs: "EPSG:4326" },
    render_authority: { region_id: "region-full-world-reverse", revision: 1 },
  };
  await withProductionMap("antimeridian-native-full-world-reverse", nzMapProps({
    bbox: { west: -180, south: 0, east: 180, north: 1, crs: "EPSG:4326" },
    aoiBbox: { west: -180, south: 0, east: 180, north: 1, crs: "EPSG:4326" },
    aoiGeometry: fullWorld,
    aoiDisplayAuthority: authority,
    scenes: [scene],
    preview: undefined,
    previewGeometry: null,
  }), async (view) => {
    assert.equal(view.container.querySelector('[data-map-primitive="polygon"][data-class-name="scene-footprint"]'), null);
  });
});

test("production WorkbenchMap hides a native global Preview bbox that spans two seam copies", async () => {
  const seamAoi = {
    type: "Polygon",
    coordinates: [[[179, -1], [-179, -1], [-179, 1], [179, 1], [179, -1]]],
  };
  const authority = {
    region_id: "region-global-preview",
    revision: 4,
    envelope: {
      south: -1,
      north: 1,
      crossesAntimeridian: false,
      displayWest: -180,
      displayEast: 180,
      requestWindows: [{ west: -180, south: -1, east: 180, north: 1, crs: "EPSG:4326" }],
    },
    render_geometry: {
      type: "MultiPolygon",
      coordinates: [
        [[[179, -1], [180, -1], [180, 1], [179, 1], [179, -1]]],
        [[[-180, -1], [-179, -1], [-179, 1], [-180, 1], [-180, -1]]],
      ],
    },
    render_bbox: { west: -180, south: -1, east: 180, north: 1 },
  };
  await withProductionMap("antimeridian-native-global-preview-wide", nzMapProps({
    aoiGeometry: seamAoi,
    aoiDisplayAuthority: authority,
    previewGeometry: null,
    previewRenderAuthority: {
      region_id: "region-global-preview",
      revision: 4,
      render_geometry: {
        type: "MultiPolygon",
        coordinates: [
          [[[-181, -1], [-179, -1], [-179, 1], [-181, 1], [-181, -1]]],
          [[[179, -1], [181, -1], [181, 1], [179, 1], [179, -1]]],
        ],
      },
      render_bbox: { west: -181, south: -1, east: 181, north: 1 },
    },
  }), async (view) => {
    assert.equal(view.container.querySelector('[data-map-primitive="image-overlay"]'), null);
  });
});

test("production WorkbenchMap hides canonical wide bbox-only Preview instead of drawing 358 degrees", async () => {
  const globalAoi = {
    type: "Polygon",
    coordinates: [[[-180, -90], [180, -90], [180, 90], [-180, 90], [-180, -90]]],
  };
  await withProductionMap("antimeridian-wide-preview-hide", nzMapProps({
    bbox: { west: -180, south: -90, east: 180, north: 90, crs: "EPSG:4326" },
    aoiBbox: { west: -180, south: -90, east: 180, north: 90, crs: "EPSG:4326" },
    aoiGeometry: globalAoi,
    scenes: [],
    previewBbox: { west: -179, south: -10, east: 179, north: 10, crs: "EPSG:4326" },
    previewGeometry: null,
  }), async (view) => {
    assert.equal(view.container.querySelector('[data-map-primitive="image-overlay"]'), null);
  });
});

test("production WorkbenchMap keeps explicit scene focus extent outside the AOI domain", async () => {
  const extendedSceneBbox = { west: 150, south: -45, east: 200, north: -35, crs: "EPSG:4326" };
  const authority = {
    region_id: "region-nz",
    revision: 2,
    envelope: {
      south: -47,
      north: -34,
      crossesAntimeridian: true,
      displayWest: 166,
      displayEast: 185,
      requestWindows: [
        { west: 166, south: -47, east: 180, north: -34, crs: "EPSG:4326" },
        { west: -180, south: -47, east: -175, north: -34, crs: "EPSG:4326" },
      ],
    },
    render_geometry: nzAoiGeometry,
    render_bbox: { west: 166, south: -47, east: 185, north: -34 },
  };
  await withProductionMap("antimeridian-focus-outside-aoi", nzMapProps({
    bbox: extendedSceneBbox,
    aoiDisplayAuthority: authority,
    scenes: [],
    preview: undefined,
    previewGeometry: null,
  }), async (_view, boundary) => {
    const fit = boundary.boundaryCapture().fitBounds.at(-1) as [[number, number], [number, number]] | undefined;
    assert.deepEqual(fit, [[-45, 150], [-35, 200]]);
  });
});

test("production WorkbenchMap rejects a 358-degree bbox-only focus for a single-window seam alias", async () => {
  const seamAliasAoi = {
    type: "Polygon",
    coordinates: [[[-170, 0], [180, 0], [180, 1], [-170, 1], [-170, 0]]],
  };
  await withProductionMap("antimeridian-seam-alias-focus", nzMapProps({
    bbox: { west: -179, south: 0, east: 179, north: 1, crs: "EPSG:4326" },
    aoiBbox: { west: -180, south: 0, east: -170, north: 1, crs: "EPSG:4326" },
    aoiGeometry: seamAliasAoi,
    scenes: [],
    preview: undefined,
    previewGeometry: null,
  }), async (view, boundary) => {
    const fit = boundary.boundaryCapture().fitBounds.at(-1) as [[number, number], [number, number]] | undefined;
    assert.deepEqual(fit, [[0, -180], [1, -170]]);
    assert.ok((fit![1][1] - fit![0][1]) < 30);
    assert.equal(view.container.querySelector('[data-map-primitive="image-overlay"]'), null);
  });
});

test("production WorkbenchMap keeps a global seam scene on the honest raw fallback", async () => {
  const globalAoi = {
    type: "Polygon",
    coordinates: [[[-180, -90], [180, -90], [180, 90], [-180, 90], [-180, -90]]],
  };
  await withProductionMap("antimeridian-global-fallback", nzMapProps({
    bbox: { west: -180, south: -90, east: 180, north: 90, crs: "EPSG:4326" },
    aoiBbox: { west: -180, south: -90, east: 180, north: 90, crs: "EPSG:4326" },
    aoiGeometry: globalAoi,
    scenes: [nzScene],
    preview: undefined,
    previewGeometry: null,
  }), async (view, boundary) => {
    const aoiLongitudes = longitudes(view, '[data-map-primitive="polygon"][data-class-name="aoi-overlay"]');
    assert.ok(aoiLongitudes.every((value) => value >= -180 && value <= 180));
    const sceneLongitudes = longitudes(view, '[data-map-primitive="polygon"][data-class-name="scene-footprint"]');
    assert.ok(sceneLongitudes.includes(-179));
    assert.ok(sceneLongitudes.includes(179));
    assert.ok(sceneLongitudes.every((value) => value >= -180 && value <= 180));
    const fit = boundary.boundaryCapture().fitBounds.at(-1) as [[number, number], [number, number]] | undefined;
    assert.deepEqual(fit, [[-90, -180], [90, 180]]);
  });
});

test("production WorkbenchMap preserves scene click selection after render projection", async () => {
  let selected: string | null = null;
  await withProductionMap("antimeridian-selection", nzMapProps({
    onSceneSelect: (sceneId: string) => { selected = sceneId; },
    preview: undefined,
    previewGeometry: null,
  }), async (view) => {
    const scene = view.container.querySelector('[data-map-primitive="polygon"][data-class-name="scene-footprint"]') as HTMLElement | null;
    assert.ok(scene, "production scene polygon was not rendered");
    scene.click();
    assert.equal(selected, "scene-nz");
  });
});

test("production map trash dispatches one explicit spatial-presentation clear action", async () => {
  let clearCount = 0;
  await withProductionMap("typed-map-spatial-clear", nzMapProps({
    onClearLayers: () => { clearCount += 1; },
  }), async (view) => {
    const clearButton = view.container.querySelector('button[title="删除所有图层"]') as HTMLButtonElement | null;
    assert.ok(clearButton, "map trash control must remain reachable");
    clearButton.click();
    assert.equal(clearCount, 1);
  });
});
