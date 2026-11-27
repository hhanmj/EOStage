import "../../test-utils/setup-dom";
import assert from "node:assert/strict";
import test from "node:test";
import React from "react";
import { fireEvent, waitFor } from "@testing-library/react";
import { cleanup, render } from "../../test-utils/render-harness";
import type { SarProduct } from "../../lib/generated/v3-contracts.ts";
import { projectSarProductToScene } from "../../lib/sarProduct.ts";
import {
  clearTestDesktopShellBridge,
  registerTestDesktopShellBridge,
} from "../../lib/desktopShellBridge.ts";
import { createWorkbenchTestDesktopShellBridge } from "../../test-utils/workbenchDesktopBridge.ts";
import { useWorkbenchMapPreview, canSceneShowMapPreview } from "../../hooks/useWorkbenchMapPreview.ts";
import { TopCenterToastHost, clearAllTopToastsForTest } from "../ui/TopCenterToast.tsx";
import { ThumbnailSlot } from "./ThumbnailSlot.tsx";
import { ManualWorkbench } from "./ManualWorkbench.tsx";
import { I18nProvider } from "../../i18n/index.ts";
import type { Bbox } from "../../lib/bridge.ts";

const originalIntersectionObserver = globalThis.IntersectionObserver;

test.afterEach(() => {
  cleanup();
  clearAllTopToastsForTest();
  clearTestDesktopShellBridge();
  Object.defineProperty(globalThis, "IntersectionObserver", {
    configurable: true,
    writable: true,
    value: originalIntersectionObserver,
  });
});

test("a visible optical result slot resolves its MPC quicklook and renders the image", async () => {
  const productKey = "PROVIDER::copernicus.dataspace.odata::COLLECTION::SENTINEL-2::ITEM::165ba4c9-997c-4cea-970f-33ab9bdb8084";
  const mapItemId = "S2B_MSIL2A_20250129T022849_R046_T51RUH_20250129T040545";
  const scene = projectSarProductToScene({
    product_key: productKey,
    provider_product_key: productKey,
    provider_product_id: "165ba4c9-997c-4cea-970f-33ab9bdb8084",
    product_id: "165ba4c9-997c-4cea-970f-33ab9bdb8084",
    scene_id: "S2B_MSIL2A_20250129T022849_N0511_R046_T51RUH_20250129T040545.SAFE",
    provider: "copernicus.dataspace.odata",
    dataset: "SENTINEL-2 L2A",
    footprint_geojson: {
      type: "Polygon",
      coordinates: [[[110, 30], [111, 30], [111, 31], [110, 31], [110, 30]]],
    },
    native_metadata: {
      logical_dataset: "optical.sentinel-2.l2a",
      native_collection_id: "SENTINEL-2",
      source_kind: "sentinel2",
      product_kind: "optical_scene",
    },
  } as SarProduct);
  const calls: Array<{ request_id: string; batch_id: string; item_id: string }> = [];
  const base = createWorkbenchTestDesktopShellBridge();
  registerTestDesktopShellBridge({
    ...base,
    application: {
      ...base.application,
      sentinel2MapPreview: async (request) => {
        calls.push(request);
        return { ok: true, preview: { product_key: productKey, map_item_id: mapItemId } };
      },
    },
  });
  class VisibleIntersectionObserver {
    private readonly callback: IntersectionObserverCallback;
    constructor(callback: IntersectionObserverCallback) {
      this.callback = callback;
    }
    observe(target: Element) {
      queueMicrotask(() => this.callback([
        { isIntersecting: true, target } as IntersectionObserverEntry,
      ], this as unknown as IntersectionObserver));
    }
    disconnect() {}
    unobserve() {}
  }
  Object.defineProperty(globalThis, "IntersectionObserver", {
    configurable: true,
    writable: true,
    value: VisibleIntersectionObserver,
  });

  const view = render(<ThumbnailSlot scene={scene} batchId="batch-optical-1" />);
  const image = await waitFor(() => view.getByRole("img", { name: "快视图" }) as HTMLImageElement);
  // Owner 2026-11-14 #6: replaced clipped Z=8 single-corner tile quicklooks with centered whole-scene Planetary Computer preview.png (max_size=192).
  assert.match(image.src, /^https:\/\/planetarycomputer\.microsoft\.com\/api\/data\/v1\/item\/preview\.png\?/);
  assert.match(image.src, /collection=sentinel-2-l2a/);
  assert.match(image.src, /format=png/);
  assert.match(image.src, /max_size=192/);
  assert.match(image.src, /item=S2B_MSIL2A_20250129T022849_R046_T51RUH_20250129T040545/);
  assert.equal(calls.length, 1);
  assert.equal(calls[0]!.batch_id, "batch-optical-1");
  assert.equal(calls[0]!.item_id, productKey);
  assert.match(calls[0]!.request_id, /^optical-quicklook-/);
});

test("a Landsat row without an authenticated USGS session shows failure without calling browse", async () => {
  const productKey = "PROVIDER::usgs.eros.m2m::COLLECTION::landsat_ot_c2_l2::ITEM::LC09_L2SP_123045_20260101_02_T1";
  const scene = projectSarProductToScene({
    product_key: productKey,
    provider_product_key: productKey,
    provider_product_id: "LC09_L2SP_123045_20260101_02_T1",
    product_id: "LC09_L2SP_123045_20260101_02_T1",
    scene_id: "LC09_L2SP_123045_20260101_20260103_02_T1",
    provider: "usgs.eros.m2m",
    dataset: "LANDSAT 8-9 C2 L2",
    native_metadata: {
      logical_dataset: "optical.landsat.c2.l2",
      native_collection_id: "landsat_ot_c2_l2",
      source_kind: "landsat",
      product_kind: "optical_scene",
      properties: { entity_id: "LC09_L2SP_123045_20260101_02_T1", browse_available: true },
    },
  } as SarProduct);
  let statusCalls = 0;
  let browseCalls = 0;
  const base = createWorkbenchTestDesktopShellBridge();
  registerTestDesktopShellBridge({
    ...base,
    application: {
      ...base.application,
      usgsSessionStatus: async () => {
        statusCalls += 1;
        return { ok: true, status: { state: "logged_out", authenticated: false } };
      },
      landsatBrowse: async () => {
        browseCalls += 1;
        throw new Error("must not be called");
      },
    },
  });
  class VisibleIntersectionObserver {
    private readonly callback: IntersectionObserverCallback;
    constructor(callback: IntersectionObserverCallback) {
      this.callback = callback;
    }
    observe(target: Element) {
      queueMicrotask(() => this.callback([
        { isIntersecting: true, target } as IntersectionObserverEntry,
      ], this as unknown as IntersectionObserver));
    }
    disconnect() {}
    unobserve() {}
  }
  Object.defineProperty(globalThis, "IntersectionObserver", {
    configurable: true,
    writable: true,
    value: VisibleIntersectionObserver,
  });

  const view = render(<ThumbnailSlot scene={scene} batchId="batch-landsat-no-auth" />);
  await waitFor(() => view.getByLabelText("快视图加载失败"));
  assert.equal(statusCalls, 1);
  assert.equal(browseCalls, 0);
  view.rerender(<ThumbnailSlot scene={scene} batchId="batch-landsat-no-auth" />);
  await waitFor(() => view.getByLabelText("快视图加载失败"));
  assert.equal(statusCalls, 1);
});

test("Sentinel-2 L1C pinned map preview resolves directly and ThumbnailSlot survives cloned scene rerenders", async () => {
  const l1cUuid = "3a1ec453-05ae-4aef-ac3e-148e7c5b412b";
  const productKey = `PROVIDER::copernicus.dataspace.odata::COLLECTION::SENTINEL-2::ITEM::${l1cUuid}`;
  const scene = projectSarProductToScene({
    product_key: productKey,
    provider_product_key: productKey,
    provider_product_id: l1cUuid,
    product_id: l1cUuid,
    scene_id: "S2A_MSIL1C_20261116T030141_N0513_R032_T50SMA_20261116T045615.SAFE",
    provider: "copernicus.dataspace.odata",
    dataset: "SENTINEL-2 L1C",
    footprint_geojson: {
      type: "Polygon",
      coordinates: [[[117.0, 31.6], [117.6, 31.6], [117.6, 32.1], [117.0, 32.1], [117.0, 31.6]]],
    },
    native_metadata: {
      logical_dataset: "optical.sentinel-2.l1c",
      native_collection_id: "SENTINEL-2",
      source_kind: "sentinel2",
      product_kind: "optical_scene",
    },
  } as SarProduct);

  let cdseCalls = 0;
  let resolveSlowThumb!: (value: { ok: true; quicklook: { media_type: string; data_url: string; byte_length: number } }) => void;
  const base = createWorkbenchTestDesktopShellBridge();
  registerTestDesktopShellBridge({
    ...base,
    application: {
      ...base.application,
      cdseQuicklook: async () => {
        cdseCalls += 1;
        return new Promise((resolve) => {
          resolveSlowThumb = resolve;
        });
      },
    },
  });

  class VisibleIntersectionObserver {
    private readonly callback: IntersectionObserverCallback;
    constructor(callback: IntersectionObserverCallback) {
      this.callback = callback;
    }
    observe(target: Element) {
      queueMicrotask(() =>
        this.callback([{ isIntersecting: true, target } as IntersectionObserverEntry], this as unknown as IntersectionObserver)
      );
    }
    disconnect() {}
    unobserve() {}
  }
  Object.defineProperty(globalThis, "IntersectionObserver", {
    configurable: true,
    writable: true,
    value: VisibleIntersectionObserver,
  });

  let mapResolveCalls = 0;
  function Harness({ rowScene }: { rowScene: typeof scene }) {
    const preview = useWorkbenchMapPreview({
      scenes: [rowScene],
      batchId: "batch-l1c-preview-1",
      explicitToggleMode: true,
      resolveOpticalQuicklook: async (req) => {
        mapResolveCalls += 1;
        assert.equal(req.scene.scene_id, scene.scene_id);
        return "data:image/jpeg;base64,L1CMAP";
      },
    });
    return (
      <div>
        <ThumbnailSlot scene={rowScene} batchId="batch-l1c-preview-1" />
        <button
          type="button"
          data-testid="pin-l1c"
          onClick={() => preview.togglePinPreviewScene(rowScene)}
        >
          pin
        </button>
        <div data-testid="pinned-count">{preview.pinnedPreviews.length}</div>
        <div data-testid="pinned-url">{preview.pinnedPreviews[0]?.url ?? ""}</div>
      </div>
    );
  }

  const view = render(<Harness rowScene={scene} />);
  await waitFor(() => assert.equal(cdseCalls, 1));

  // Re-render with cloned scene object while CDSE quicklook is still in-flight: must NOT cancel/re-enqueue.
  view.rerender(<Harness rowScene={{ ...scene }} />);
  fireEvent.click(view.getByTestId("pin-l1c"));

  await waitFor(() => assert.equal(view.getByTestId("pinned-count").textContent, "1"));
  assert.equal(view.getByTestId("pinned-url").textContent, "data:image/jpeg;base64,L1CMAP");
  assert.equal(mapResolveCalls, 1);

  resolveSlowThumb({
    ok: true,
    quicklook: {
      media_type: "image/jpeg",
      data_url: "data:image/jpeg;base64,L1CTHUMB",
      byte_length: 8,
    },
  });
  const img = await waitFor(() => view.getByRole("img", { name: "快视图" }) as HTMLImageElement);
  assert.equal(img.src, "data:image/jpeg;base64,L1CTHUMB");
  assert.equal(img.className.includes("object-contain"), true);
  assert.equal(cdseCalls, 1);
});

test("canSceneShowMapPreview gates rows honestly and handlePinnedPreviewError falls back or unpins with toast", async () => {
  const bhoonidhiThumbUrl = "https://bhoonidhi.nrsc.gov.in/stac/PRODUCTJPGS/NISAR/SSAR/test.jpg";
  const bhoonidhiScene = projectSarProductToScene({
    product_key: "PROVIDER::isro.nrsc.bhoonidhi::COLLECTION::NISAR_SSAR_RSLC::ITEM::NISAR_S1_UR_RSLC_TEST",
    provider_product_key: "PROVIDER::isro.nrsc.bhoonidhi::COLLECTION::NISAR_SSAR_RSLC::ITEM::NISAR_S1_UR_RSLC_TEST",
    provider_product_id: "NISAR_S1_UR_RSLC_TEST",
    product_id: "NISAR_S1_UR_RSLC_TEST",
    scene_id: "2026310234512_26_18",
    provider: "isro.nrsc.bhoonidhi",
    dataset: "sar.nisar.s.ssar.rslc.v1",
    footprint_bbox: { west: 80, south: 31, east: 83, north: 34, crs: "EPSG:4326" },
    assets: [{
      role: "thumbnail",
      url: bhoonidhiThumbUrl,
      file_name: "test.jpg",
      media_type: "image/jpeg",
      size_bytes: null,
    }],
    native_metadata: { logical_dataset: "sar.nisar.s.ssar.rslc.v1" },
  } as SarProduct);

  assert.equal(canSceneShowMapPreview(bhoonidhiScene, "batch-bhoonidhi-1"), true);
  assert.equal(
    canSceneShowMapPreview({ ...bhoonidhiScene, assets: [], sar_product: undefined, native_metadata: {} }, "batch-bhoonidhi-1"),
    false,
  );

  let mode: "succeed" | "fail" = "succeed";
  const base = createWorkbenchTestDesktopShellBridge();
  registerTestDesktopShellBridge({
    ...base,
    application: {
      ...base.application,
      previewAsset: async () => {
        if (mode === "fail") throw new Error("fetch failed");
        return {
          ok: true,
          preview: {
            product_key: bhoonidhiScene.product_key,
            descriptor: {
              render_profile_id: "test",
              credential_requirement: "none",
              resolution_tier: "browse",
            },
            source_asset_role: "thumbnail",
            source_asset_key: "thumbnail",
            media_type: "image/jpeg",
            data_url: "data:image/jpeg;base64,FALLBACK_OK",
            byte_length: 11,
          },
        } as any;
      },
    },
  });

  function Harness() {
    const preview = useWorkbenchMapPreview({
      scenes: [bhoonidhiScene],
      batchId: "batch-bhoonidhi-1",
      explicitToggleMode: true,
    });
    return (
      <div>
        <TopCenterToastHost />
        <button type="button" data-testid="pin-btn" onClick={() => preview.togglePinPreviewScene(bhoonidhiScene)}>
          pin
        </button>
        <button
          type="button"
          data-testid="err-btn"
          onClick={() => preview.handlePinnedPreviewError(bhoonidhiScene.scene_id)}
        >
          err
        </button>
        <div data-testid="pinned-size">{preview.pinnedPreviewIds.size}</div>
        <div data-testid="pinned-url">{preview.pinnedPreviews[0]?.url ?? ""}</div>
      </div>
    );
  }

  const view = render(<Harness />);
  fireEvent.click(view.getByTestId("pin-btn"));
  assert.equal(view.getByTestId("pinned-size").textContent, "1");
  assert.equal(view.getByTestId("pinned-url").textContent, bhoonidhiThumbUrl);

  // First image error triggers backend getPreviewAsset fallback cleanly
  fireEvent.click(view.getByTestId("err-btn"));
  await waitFor(() => assert.equal(view.getByTestId("pinned-url").textContent, "data:image/jpeg;base64,FALLBACK_OK"));
  assert.equal(view.getByTestId("pinned-size").textContent, "1");

  // Second error (no further fallback) unpins and shows toast
  mode = "fail";
  fireEvent.click(view.getByTestId("err-btn"));
  await waitFor(() => assert.equal(view.getByTestId("pinned-size").textContent, "0"));
  assert.ok(view.getByTestId("top-center-toast").textContent?.includes("该影像快视图加载失败"));
});

test("canSceneShowMapPreview and useWorkbenchMapPreview prefer safe preview_data_url when S3 thumbnail host is CSP-blocked, and unpin without batchId", async () => {
  const blockedS3Url = "https://sentinel-s1-l1c.s3.eu-central-1.amazonaws.com/GRD/2026/11/24/preview/quick-look.png";
  const safeDataUrl = "data:image/png;base64,iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mNk+M9QDwADhgGAWjR9awAAAABJRU5ErkJggg==";

  const awsGrdWithDataUrl = {
    product_key: "S1D_IW_GRDH_1SDV_TEST",
    product_id: "S1D_IW_GRDH_1SDV_TEST",
    provider: "AWS Open Data",
    dataset: "science.aws.sentinel-1-grd.v1",
    file_name: "S1D_IW_GRDH_1SDV_TEST (VV COG)",
    scene_id: "S1D_IW_GRDH_1SDV_TEST",
    platform: "Sentinel-1",
    product_type: "GRD VV",
    beam_mode: "",
    polarization: "VV",
    acquisition_datetime: "2026-11-24T10:11:18Z",
    absolute_orbit: null,
    relative_orbit: null,
    path: null,
    frame: null,
    orbit_direction: "ASCENDING",
    footprint_bbox: { west: 115.9, south: 31.4, east: 118.8, north: 33.3, crs: "EPSG:4326" as const },
    footprint_geojson: null,
    has_url: true,
    download_url: "https://example.invalid/cog.tif",
    assets: [
      {
        role: "thumbnail",
        url: blockedS3Url,
        file_name: "quick-look.png",
        media_type: "image/png",
        size_bytes: null,
      },
    ],
    native_metadata: {
      thumbnail_url: blockedS3Url,
      preview_url: blockedS3Url,
      preview_data_url: safeDataUrl,
    },
  };

  const awsGrdWithoutDataUrl = {
    ...awsGrdWithDataUrl,
    scene_id: "S1D_IW_GRDH_NO_DATA_URL",
    product_key: "S1D_IW_GRDH_NO_DATA_URL",
    product_id: "S1D_IW_GRDH_NO_DATA_URL",
    native_metadata: {
      thumbnail_url: blockedS3Url,
      preview_url: blockedS3Url,
    },
  };

  assert.equal(canSceneShowMapPreview(awsGrdWithDataUrl as any, null), true);
  assert.equal(canSceneShowMapPreview(awsGrdWithoutDataUrl as any, null), true);

  const untrustedUrl = "https://untrusted.example.invalid/GRD/quick-look.png";
  const untrustedScene = {
    ...awsGrdWithoutDataUrl,
    scene_id: "S1D_IW_GRDH_UNTRUSTED",
    product_key: "S1D_IW_GRDH_UNTRUSTED",
    product_id: "S1D_IW_GRDH_UNTRUSTED",
    assets: [
      {
        role: "thumbnail",
        url: untrustedUrl,
        file_name: "quick-look.png",
        media_type: "image/png",
        size_bytes: null,
      },
    ],
    native_metadata: {
      thumbnail_url: untrustedUrl,
      preview_url: untrustedUrl,
    },
  };
  assert.equal(canSceneShowMapPreview(untrustedScene as any, null), false);

  class VisibleIntersectionObserver {
    private readonly callback: IntersectionObserverCallback;
    constructor(callback: IntersectionObserverCallback) {
      this.callback = callback;
    }
    observe(target: Element) {
      queueMicrotask(() =>
        this.callback(
          [{ isIntersecting: true, target } as IntersectionObserverEntry],
          this as unknown as IntersectionObserver,
        ),
      );
    }
    disconnect() {}
    unobserve() {}
  }
  Object.defineProperty(globalThis, "IntersectionObserver", {
    configurable: true,
    writable: true,
    value: VisibleIntersectionObserver,
  });

  const proxiedDataUrl = "data:image/png;base64,PROXIED_AWS_S1";
  let proxyCalls = 0;
  const baseBridge = createWorkbenchTestDesktopShellBridge();
  registerTestDesktopShellBridge({
    ...baseBridge,
    application: {
      ...baseBridge.application,
      fetchGeodataPreviewDataUrl: async (url: string) => {
        proxyCalls += 1;
        assert.equal(url, blockedS3Url);
        return { ok: true, data_url: proxiedDataUrl };
      },
    },
  });

  function Harness({ scene }: { scene: typeof awsGrdWithDataUrl }) {
    const preview = useWorkbenchMapPreview({
      scenes: [scene as any],
      batchId: null,
      explicitToggleMode: true,
    });
    return (
      <div>
        <TopCenterToastHost />
        <ThumbnailSlot scene={scene as any} batchId={null} />
        <button type="button" data-testid="pin-aws-btn" onClick={() => preview.togglePinPreviewScene(scene as any)}>
          pin
        </button>
        <div data-testid="aws-pinned-size">{preview.pinnedPreviewIds.size}</div>
        <div data-testid="aws-pinned-url">{preview.pinnedPreviews[0]?.url ?? ""}</div>
      </div>
    );
  }

  const view = render(<Harness scene={awsGrdWithDataUrl} />);
  fireEvent.click(view.getByTestId("pin-aws-btn"));
  assert.equal(view.getByTestId("aws-pinned-size").textContent, "1");
  assert.equal(view.getByTestId("aws-pinned-url").textContent, safeDataUrl);

  // Proxyable S1 AWS scene without pre-hydrated preview_data_url resolves both ThumbnailSlot and pinned map preview via backend proxy
  cleanup();
  const proxyView = render(<Harness scene={awsGrdWithoutDataUrl as any} />);
  const thumbImg = await waitFor(() => proxyView.getByRole("img", { name: "快视图" }) as HTMLImageElement);
  assert.equal(thumbImg.src, proxiedDataUrl);
  assert.equal(proxyCalls, 1);

  fireEvent.click(proxyView.getByTestId("pin-aws-btn"));
  await waitFor(() => assert.equal(proxyView.getByTestId("aws-pinned-size").textContent, "1"));
  assert.equal(proxyView.getByTestId("aws-pinned-url").textContent, proxiedDataUrl);

  // Guard: GEE scene IDs are handled independently by onPreviewScene and never enter pinned preview / emit failure toast
  const geeScene = {
    ...awsGrdWithoutDataUrl,
    scene_id: "gee-scene::COPERNICUS/S2_SR_HARMONIZED/20261120",
    product_key: "gee-scene::COPERNICUS/S2_SR_HARMONIZED/20261120",
    product_id: "gee-scene::COPERNICUS/S2_SR_HARMONIZED/20261120",
    dataset: "science.gee.catalog.v1",
  };
  cleanup();
  const geeView = render(<Harness scene={geeScene as any} />);
  fireEvent.click(geeView.getByTestId("pin-aws-btn"));
  await new Promise((r) => setTimeout(r, 20));
  assert.equal(geeView.getByTestId("aws-pinned-size").textContent, "0");
  assert.equal(geeView.queryByTestId("top-center-toast") === null, true);
});

test("ManualWorkbench passes streaming batchDelivery.batchId to rows and preview hook before activeSourceView hydrates", () => {
  const s2Uuid = "8d91c512-0c5e-4e92-84c6-214e3af1899a";
  const productKey = `PROVIDER::copernicus.dataspace.odata::COLLECTION::SENTINEL-2::ITEM::${s2Uuid}`;
  const s2Scene = projectSarProductToScene({
    product_key: productKey,
    provider_product_key: productKey,
    provider_product_id: s2Uuid,
    product_id: s2Uuid,
    scene_id: "S2C_MSIL2A_20261124T030031_N0513_R032_T50SNB_20261124T062550.SAFE",
    provider: "copernicus.dataspace.odata",
    dataset: "SENTINEL-2 L2A",
    footprint_geojson: {
      type: "Polygon",
      coordinates: [[[117.0, 31.6], [117.6, 31.6], [117.6, 32.1], [117.0, 32.1], [117.0, 31.6]]],
    },
    native_metadata: {
      logical_dataset: "optical.sentinel-2.l2a",
      native_collection_id: "SENTINEL-2",
      source_kind: "sentinel2",
      product_kind: "optical_scene",
    },
  } as SarProduct);

  const bbox: Bbox = { west: 117.0, south: 31.6, east: 117.6, north: 32.1, crs: "EPSG:4326" };
  const view = render(
    <I18nProvider>
      <div style={{ width: 1440 }}>
        <ManualWorkbench
          layoutWidth={1440}
          scientificMode="sar"
          onScientificModeChange={() => {}}
          onResetParams={() => {}}
          availableSources={[{ resource_id: "science.sentinel-2.l2a.v1", display_name_zh: "Sentinel-2 L2A", family: "optical" } as any]}
          activeResourceId="science.sentinel-2.l2a.v1"
          onSelectSource={() => {}}
          productType=""
          onProductTypeChange={() => {}}
          workingAoi={bbox}
          workingGeometry={s2Scene.footprint_geojson as Record<string, unknown>}
          aoiLabel="合肥市"
          onReplaceAoi={async () => {}}
          onImportVectorSuccess={() => {}}
          startDate="2026-10-01"
          endDate="2026-11-25"
          onStartDateChange={() => {}}
          onEndDateChange={() => {}}
          orbitDirection="all"
          onOrbitDirectionChange={() => {}}
          relativeOrbit={null}
          onRelativeOrbitChange={() => {}}
          frame={null}
          onFrameChange={() => {}}
          searching={true}
          onTriggerSearch={() => {}}
          frozenSearchRequest={null}
          activeSourceView={null}
          batchDelivery={{
            kind: "loading",
            batchId: "batch-streaming-s2-1",
            target: 250,
            matched: 288,
            loaded: 1,
            pageable: 1,
          }}
          scenes={[s2Scene]}
          resultTotalCount={288}
          selectedSceneId={null}
          onSelectScene={() => {}}
          checkedSceneIds={new Set()}
          onToggleCheckScene={() => {}}
          onToggleCheckAll={() => {}}
          onDownloadChecked={() => {}}
          downloadBusy={false}
          drawMode="rect"
          drawActive={false}
          onDrawModeChange={() => {}}
          onDrawActiveChange={() => {}}
          onClearLayers={() => {}}
          onRectDraw={() => {}}
          onPolygonDraw={() => {}}
          onPointDraw={() => {}}
          mapLayerKey="arcgisSatellite"
          onLayerChange={() => {}}
        />
      </div>
    </I18nProvider>,
  );

  // Preview button must appear immediately on initial render using batchDelivery.batchId
  // before activeSourceView.batch_id hydrates.
  assert.ok(view.getByTestId("row-preview-toggle"));
});
