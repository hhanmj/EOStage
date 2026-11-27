import "../../test-utils/setup-dom";
import assert from "node:assert/strict";
import test from "node:test";
import React from "react";
import { waitFor } from "@testing-library/react";
import { cleanup, fireEvent, render } from "../../test-utils/render-harness";
import { OpticalAoiClipModal } from "./OpticalAoiClipModal";
import type { DesktopOpticalDerivedCreateTaskRequest } from "../../lib/desktopShellBridge";

test.afterEach(() => cleanup());

const geometry = {
  type: "Polygon",
  coordinates: [[[114.2, 30.5], [114.4, 30.5], [114.4, 30.7], [114.2, 30.7], [114.2, 30.5]]],
};

test("single-scene Sentinel-2 clip modal runs precheck, warns on cross-UTM, and submits CloudToNodata clip task", async () => {
  let submitted: DesktopOpticalDerivedCreateTaskRequest | null = null;
  const view = render(
    <OpticalAoiClipModal
      sourceBatchId="batch-s2"
      expectedWorkbenchRevision={3}
      scenes={[
        {
          scene_id: "S2A_MSIL2A_20260101_T50RKU",
          provider: "microsoft.planetary-computer",
          native_metadata: { logical_dataset: "optical.sentinel-2.l2a" },
        },
      ]}
      initialOutputRoot="/tmp/optical_out"
      initialConcurrency={2}
      aoiGeometry={geometry}
      aoiLabel="武汉市"
      onClose={() => {}}
      onPrecheck={async () => ({
        ok: true,
        report: {
          ok: true,
          recommended_mode: "single_scene_clip",
          aoi_bbox_wgs84: [114.2, 30.5, 114.4, 30.7],
          aoi_center_wgs84: [114.3, 30.6],
          recommended_utm_epsg: 32650,
          crosses_utm_zones: true,
          distinct_utm_zones: [32649, 32650],
          scene_count: 1,
          intersecting_scene_count: 1,
          combined_aoi_coverage_fraction: 0.925,
          temporal_span_days: 0,
          within_near_coincident_limit: true,
          estimated_output_pixels_per_band: 4000000,
          warnings: ["cross_utm_reprojection"],
          blocking_reasons: [],
          scene_coverages: [
            {
              scene_id: "S2A_MSIL2A_20260101_T50RKU",
              intersects_aoi: true,
              aoi_coverage_fraction: 0.925,
              scene_epsg: 32650,
              acquired_at: "2026-01-01T03:00:00Z",
              cloud_cover_pct: 5.2,
            },
          ],
        },
      })}
      onSubmit={async (request) => {
        submitted = request;
      }}
    />,
  );

  await waitFor(() => view.getByTestId("optical-cross-utm-warning"));
  assert.ok(view.getAllByText(/EPSG:32650/).length >= 1);
  assert.equal(view.queryByTestId("optical-mosaic-options"), null);

  fireEvent.click(view.getByTestId("band-preset-false_color_nir"));
  fireEvent.click(view.getByTestId("cloud-mask-nodata"));
  const confirmBtn = view.getByTestId("btn-confirm-optical-aoi-clip") as HTMLButtonElement;
  assert.equal(confirmBtn.disabled, false);
  fireEvent.click(confirmBtn);

  await waitFor(() => assert.ok(submitted !== null));
  assert.equal(submitted!.mode, "single_scene_clip");
  assert.deepEqual(submitted!.band_order, ["B08", "B04", "B03"]);
  assert.equal(submitted!.cloud_mask, "cloud_to_nodata");
  assert.equal(submitted!.output_root, "/tmp/optical_out");
});

test("multi-scene Landsat C2 L2 mosaic modal exposes mosaic strategy and submits sensor band aliases", async () => {
  let submitted: DesktopOpticalDerivedCreateTaskRequest | null = null;
  const view = render(
    <OpticalAoiClipModal
      sourceBatchId="batch-landsat"
      expectedWorkbenchRevision={1}
      scenes={[
        {
          scene_id: "LC08_L2SP_123039_20260101_T1",
          provider: "microsoft.planetary-computer",
          native_metadata: { logical_dataset: "optical.landsat.c2.l2" },
        },
        {
          scene_id: "LC09_L2SP_123039_20260109_T1",
          provider: "microsoft.planetary-computer",
          native_metadata: { logical_dataset: "optical.landsat.c2.l2" },
        },
      ]}
      initialOutputRoot="/tmp/optical_out"
      initialConcurrency={2}
      aoiGeometry={geometry}
      aoiLabel="武汉市"
      onClose={() => {}}
      onPrecheck={async () => ({
        ok: true,
        report: {
          ok: true,
          recommended_mode: "near_coincident_spatial_mosaic",
          aoi_bbox_wgs84: [114.2, 30.5, 114.4, 30.7],
          aoi_center_wgs84: [114.3, 30.6],
          recommended_utm_epsg: 32650,
          crosses_utm_zones: false,
          distinct_utm_zones: [32650],
          scene_count: 2,
          intersecting_scene_count: 2,
          combined_aoi_coverage_fraction: 1.0,
          temporal_span_days: 8.0,
          within_near_coincident_limit: true,
          estimated_output_pixels_per_band: 400000,
          warnings: [],
          blocking_reasons: [],
          scene_coverages: [
            {
              scene_id: "LC08_L2SP_123039_20260101_T1",
              intersects_aoi: true,
              aoi_coverage_fraction: 0.6,
              scene_epsg: 32650,
              acquired_at: "2026-01-01T02:45:00Z",
              cloud_cover_pct: 4.0,
            },
            {
              scene_id: "LC09_L2SP_123039_20260109_T1",
              intersects_aoi: true,
              aoi_coverage_fraction: 0.55,
              scene_epsg: 32650,
              acquired_at: "2026-01-09T02:45:00Z",
              cloud_cover_pct: 8.0,
            },
          ],
        },
      })}
      onSubmit={async (request) => {
        submitted = request;
      }}
    />,
  );

  const defaultBatchRadio = await waitFor(() =>
    view.getByTestId("optical-mode-batch-single") as HTMLInputElement,
  );
  assert.equal(defaultBatchRadio.checked, true);
  assert.equal(view.queryByTestId("optical-mosaic-options"), null);
  const provCheckbox = view.getByTestId("optical-write-provenance-checkbox") as HTMLInputElement;
  assert.equal(provCheckbox.checked, false);

  fireEvent.click(view.getByTestId("optical-mode-mosaic"));
  await waitFor(() => view.getByTestId("optical-mosaic-options"));
  fireEvent.click(view.getByTestId("mosaic-strategy-latest"));
  fireEvent.click(view.getByTestId("band-preset-swir"));
  fireEvent.click(view.getByTestId("btn-confirm-optical-aoi-clip"));

  await waitFor(() => assert.ok(submitted !== null));
  assert.equal(submitted!.mode, "near_coincident_spatial_mosaic");
  assert.equal(submitted!.mosaic_strategy, "latest_first");
  assert.equal(submitted!.max_temporal_span_days, null);
  assert.equal(submitted!.write_provenance_sidecars, false);
  assert.deepEqual(submitted!.band_order, ["swir22", "nir08", "red"]);
});

test("blocked precheck (AOI_SCENE_DISJOINT / >16d) renders blocking reasons and disables submit", async () => {
  const view = render(
    <OpticalAoiClipModal
      sourceBatchId="batch-blocked"
      expectedWorkbenchRevision={1}
      scenes={[
        {
          scene_id: "S2A_DISJOINT",
          provider: "microsoft.planetary-computer",
          native_metadata: { logical_dataset: "optical.sentinel-2.l2a" },
        },
      ]}
      initialOutputRoot="/tmp/optical_out"
      initialConcurrency={2}
      aoiGeometry={geometry}
      onClose={() => {}}
      onPrecheck={async () => ({
        ok: true,
        report: {
          ok: false,
          recommended_mode: "single_scene_clip",
          aoi_bbox_wgs84: [114.2, 30.5, 114.4, 30.7],
          aoi_center_wgs84: [114.3, 30.6],
          recommended_utm_epsg: 32650,
          crosses_utm_zones: false,
          distinct_utm_zones: [32650],
          scene_count: 1,
          intersecting_scene_count: 0,
          combined_aoi_coverage_fraction: 0,
          temporal_span_days: null,
          within_near_coincident_limit: true,
          estimated_output_pixels_per_band: 0,
          warnings: [],
          blocking_reasons: ["AOI_SCENE_DISJOINT: selected scene footprint is disjoint from AOI"],
          scene_coverages: [
            {
              scene_id: "S2A_DISJOINT",
              intersects_aoi: false,
              aoi_coverage_fraction: 0,
              scene_epsg: 32650,
              acquired_at: "2026-01-01T03:00:00Z",
              cloud_cover_pct: 2.0,
            },
          ],
        },
      })}
      onSubmit={async () => {
        throw new Error("submit must remain blocked");
      }}
    />,
  );

  const blockingPanel = await waitFor(() => view.getByTestId("optical-precheck-blocking"));
  assert.ok(blockingPanel.textContent?.includes("AOI_SCENE_DISJOINT"));
  const confirmBtn = view.getByTestId("btn-confirm-optical-aoi-clip") as HTMLButtonElement;
  assert.equal(confirmBtn.disabled, true);
});

test("OpticalAoiClipModal strips OPTICAL_OUTPUT_PIXEL_LIMIT prefix, formats pixel counts, and re-runs precheck on resolution change", async () => {
  const precheckCalls: Array<{ target_resolution_m?: number | null }> = [];
  const view = render(
    <OpticalAoiClipModal
      sourceBatchId="batch-large"
      expectedWorkbenchRevision={2}
      scenes={[
        {
          scene_id: "S2A_LARGE_PIXEL_LIMIT",
          provider: "microsoft.planetary-computer",
          native_metadata: { logical_dataset: "optical.sentinel-2.l2a" },
        },
      ]}
      initialOutputRoot="/Users/demo/Downloads/EOStage"
      initialConcurrency={2}
      aoiGeometry={geometry}
      onClose={() => {}}
      onPrecheck={async (req) => {
        precheckCalls.push({ target_resolution_m: req.target_resolution_m });
        const is20m = (req.target_resolution_m ?? 10) >= 20;
        if (!is20m) {
          return {
            ok: true,
            report: {
              ok: false,
              recommended_mode: "single_scene_clip",
              aoi_bbox_wgs84: [114.2, 30.5, 114.4, 30.7],
              aoi_center_wgs84: [114.3, 30.6],
              recommended_utm_epsg: 32650,
              crosses_utm_zones: false,
              distinct_utm_zones: [32650],
              scene_count: 1,
              intersecting_scene_count: 1,
              combined_aoi_coverage_fraction: 0.05,
              temporal_span_days: 0,
              within_near_coincident_limit: true,
              estimated_output_pixels_per_band: 216239496,
              warnings: [],
              blocking_reasons: [
                "OPTICAL_OUTPUT_PIXEL_LIMIT: 预计单波段输出像元数为 216239496，超过安全上限 64000000",
              ],
              scene_coverages: [],
            },
          };
        }
        return {
          ok: true,
          report: {
            ok: true,
            recommended_mode: "single_scene_clip",
            aoi_bbox_wgs84: [114.2, 30.5, 114.4, 30.7],
            aoi_center_wgs84: [114.3, 30.6],
            recommended_utm_epsg: 32650,
            crosses_utm_zones: false,
            distinct_utm_zones: [32650],
            scene_count: 1,
            intersecting_scene_count: 1,
            combined_aoi_coverage_fraction: 0.95,
            temporal_span_days: 0,
            within_near_coincident_limit: true,
            estimated_output_pixels_per_band: 54059874,
            warnings: [],
            blocking_reasons: [],
            scene_coverages: [
              {
                scene_id: "S2A_LARGE_PIXEL_LIMIT",
                intersects_aoi: true,
                aoi_coverage_fraction: 0.95,
                scene_epsg: 32650,
                acquired_at: "2026-01-01T03:00:00Z",
                cloud_cover_pct: 1.5,
              },
            ],
          },
        };
      }}
      onSubmit={async () => {
        throw new Error("submit must remain blocked until resolution adjusted");
      }}
    />,
  );

  const blockingPanel = await waitFor(() => view.getByTestId("optical-precheck-blocking"));
  assert.doesNotMatch(blockingPanel.textContent ?? "", /OPTICAL_OUTPUT_PIXEL_LIMIT/);
  assert.match(blockingPanel.textContent ?? "", /216,239,496.*64,000,000/);
  assert.equal(view.getByTestId("optical-est-pixels").textContent?.trim(), "216,239,496");
  assert.ok(view.getByTestId("optical-clip-scene-list").textContent?.includes("S2A_LARGE_PIXEL_LIMIT"));

  // Switch resolution to 20m -> re-triggers onPrecheck with target_resolution_m: 20 and unblocks
  fireEvent.click(view.getByTestId("optical-res-quick-20"));
  await waitFor(() => {
    assert.equal(view.getByTestId("optical-est-pixels").textContent?.trim(), "54,059,874");
    assert.equal(view.queryByTestId("optical-precheck-blocking") === null, true);
  });
  assert.ok(precheckCalls.some((c) => c.target_resolution_m === 20));
  const confirmBtn = view.getByTestId("btn-confirm-optical-aoi-clip") as HTMLButtonElement;
  assert.equal(confirmBtn.disabled, false);
});

test("OpticalAoiClipModal Item 7/8/6: Sentinel-2 L1C, unitless m pills, disabled native resampling, unchecked sidecars, and open without workbench AOI", async () => {
  let submitted: DesktopOpticalDerivedCreateTaskRequest | null = null;
  const view = render(
    <OpticalAoiClipModal
      sourceBatchId="batch-l1c"
      expectedWorkbenchRevision={1}
      scenes={[
        {
          scene_id: "S2A_MSIL1C_20260101",
          provider: "copernicus.dataspace.odata",
          footprint_bbox: { west: 117.1, south: 31.7, east: 117.3, north: 31.9, crs: "EPSG:4326" },
          native_metadata: { logical_dataset: "optical.sentinel-2.l1c" },
        },
        {
          scene_id: "S2B_MSIL1C_20260106",
          provider: "copernicus.dataspace.odata",
          footprint_bbox: { west: 117.1, south: 31.7, east: 117.3, north: 31.9, crs: "EPSG:4326" },
          native_metadata: { logical_dataset: "optical.sentinel-2.l1c" },
        },
      ]}
      initialOutputRoot="/Users/demo/Downloads/EOStage"
      initialConcurrency={2}
      onClose={() => {}}
      onPrecheck={async (req) => ({
        ok: true,
        report: {
          ok: true,
          recommended_mode: req.mode === "near_coincident_spatial_mosaic" ? "near_coincident_spatial_mosaic" : "batch_single_scene_clip",
          aoi_bbox_wgs84: req.aoi_bbox_wgs84 ?? [117.1, 31.7, 117.3, 31.9],
          aoi_center_wgs84: [117.2, 31.8],
          recommended_utm_epsg: 32650,
          crosses_utm_zones: false,
          distinct_utm_zones: [32650],
          scene_count: 2,
          intersecting_scene_count: 2,
          combined_aoi_coverage_fraction: 1.0,
          temporal_span_days: 5,
          within_near_coincident_limit: true,
          estimated_output_pixels_per_band: 100000,
          warnings: [],
          blocking_reasons: [],
          scene_coverages: [],
        },
      })}
      onSubmit={async (request) => {
        submitted = request;
      }}
    />,
  );

  await waitFor(() => assert.equal((view.getByTestId("btn-confirm-optical-aoi-clip") as HTMLButtonElement).disabled, false));
  assert.ok(view.getByText("输出像元分辨率 m"));
  assert.equal(view.getByTestId("optical-res-quick-10").textContent?.trim(), "10");
  assert.equal(view.getByTestId("optical-res-quick-20").textContent?.trim(), "20");
  assert.equal(view.getByTestId("optical-res-quick-30").textContent?.trim(), "30");
  assert.equal(view.getByTestId("optical-res-quick-60").textContent?.trim(), "60");

  // Native single-resolution (10m RGB) clip disables resampling dropdown
  const resamplingSelect = view.getByTestId("optical-resampling-select") as HTMLButtonElement;
  assert.equal(resamplingSelect.disabled, true);

  // L1C disables cloud_to_nodata
  const cloudNodata = view.getByTestId("cloud-mask-nodata") as HTMLInputElement;
  assert.equal(cloudNodata.disabled, true);

  // Submitting defaults to batch_single_scene_clip with sidecars unchecked
  fireEvent.click(view.getByTestId("btn-confirm-optical-aoi-clip"));
  await waitFor(() => assert.ok(submitted !== null));
  assert.equal(submitted!.mode, "batch_single_scene_clip");
  assert.equal(submitted!.write_provenance_sidecars, false);
  assert.equal(submitted!.cloud_mask, "none");
});

test("Item 4: OpticalAoiClipModal map disables legacy Leaflet attribution, places legend bottom-left, and provides bottom-right circular i popover", async () => {
  const view = render(
    <OpticalAoiClipModal
      sourceBatchId="batch-opt-map"
      expectedWorkbenchRevision={1}
      scenes={[
        {
          scene_id: "S2A_MSIL2A_20260101_T50RKU",
          provider: "microsoft.planetary-computer",
          footprint_bbox: { west: 114.1, south: 30.4, east: 114.6, north: 30.8, crs: "EPSG:4326" },
          native_metadata: { logical_dataset: "optical.sentinel-2.l2a" },
        },
      ]}
      initialOutputRoot="/data/eostage-test-output"
      initialConcurrency={2}
      aoiGeometry={geometry}
      aoiLabel="武汉市"
      onClose={() => {}}
      onPrecheck={async () => ({
        ok: true,
        report: {
          ok: true,
          recommended_mode: "single_scene_clip",
          aoi_bbox_wgs84: [114.2, 30.5, 114.4, 30.7],
          aoi_center_wgs84: [114.3, 30.6],
          recommended_utm_epsg: 32650,
          crosses_utm_zones: false,
          distinct_utm_zones: [32650],
          scene_count: 1,
          intersecting_scene_count: 1,
          combined_aoi_coverage_fraction: 1.0,
          temporal_span_days: 0,
          within_near_coincident_limit: true,
          estimated_output_pixels_per_band: 100000,
          warnings: [],
          blocking_reasons: [],
          scene_coverages: [],
        },
      })}
      onSubmit={async () => {}}
    />,
  );

  assert.equal(view.container.querySelector(".leaflet-control-attribution"), null);
  const legend = view.getByTestId("aoi-draw-map-legend");
  assert.ok(legend.className.includes("bottom-3"));
  assert.ok(legend.className.includes("left-3"));
  assert.equal(legend.className.includes("right-3"), false);

  const attrBtn = view.getByTestId("aoi-draw-map-attribution-btn");
  assert.ok(attrBtn.className.includes("rounded-full"));
  fireEvent.click(attrBtn);
  const popover = view.getByTestId("aoi-draw-map-attribution-popover");
  assert.ok(popover.textContent?.includes("Leaflet"));
});
